//! Ghidra headless decompilation.
//!
//! This module runs Ghidra's `analyzeHeadless` with a small Java post-script,
//! imports the game binary into a temporary project, decompiles exactly one
//! function, and parses the script's result file. There is no placeholder
//! pseudocode: if Ghidra fails or produces no result, the command fails.

use crate::game::GameInfo;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::{info, warn};

/// Ghidra's default project name inside the temporary project directory.
const GHIDRA_PROJECT_NAME: &str = "GameOptProject";

/// Hard limit for a single headless decompilation.
const GHIDRA_TIMEOUT: Duration = Duration::from_secs(600);

/// The Java post-script embedded in the binary.
///
/// It writes a line-oriented result file instead of relying on Ghidra's log
/// output, which also contains analysis messages. The format is intentionally
/// simple so the Rust side can parse it without a Java/JSON dependency.
const DECOMPILE_SCRIPT: &str = r#"
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.Instruction;
import ghidra.program.model.listing.InstructionIterator;
import ghidra.program.model.listing.Parameter;
import ghidra.util.task.ConsoleTaskMonitor;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;

public class GameOptDecompile extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 2) {
            throw new IllegalArgumentException("usage: <address> <output-file>");
        }

        Address address = toAddr(args[0]);
        if (address == null) {
            throw new RuntimeException("Invalid address: " + args[0]);
        }

        Function function = getFunctionAt(address);
        if (function == null) {
            function = getFunctionContaining(address);
        }
        if (function == null) {
            throw new RuntimeException("No function found at " + address);
        }

        DecompInterface decompiler = new DecompInterface();
        decompiler.toggleCCode(true);
        decompiler.toggleSyntaxTree(true);
        decompiler.setSimplificationStyle("decompile");
        if (!decompiler.openProgram(currentProgram)) {
            throw new RuntimeException("Could not open program in decompiler");
        }

        DecompileResults results = decompiler.decompileFunction(
            function, 120, new ConsoleTaskMonitor());
        if (!results.decompileCompleted()) {
            throw new RuntimeException("Decompilation failed: " + results.getErrorMessage());
        }

        String pseudocode = results.getDecompiledFunction().getC();
        String signature = function.getPrototypeString(false, false);
        String returnType = function.getReturnType().getName();

        try (PrintWriter out = new PrintWriter(args[1], StandardCharsets.UTF_8)) {
            out.println("GAME_OPT_NAME\t" + clean(function.getName()));
            out.println("GAME_OPT_ENTRY\t" + function.getEntryPoint());
            out.println("GAME_OPT_SIGNATURE\t" + clean(signature));
            out.println("GAME_OPT_RETURN\t" + clean(returnType));
            for (Parameter parameter : function.getParameters()) {
                out.println("GAME_OPT_PARAM\t" + clean(
                    parameter.getDataType().getName() + " " + parameter.getName()));
            }

            out.println("GAME_OPT_PSEUDO_BEGIN");
            out.println(pseudocode);
            out.println("GAME_OPT_PSEUDO_END");

            out.println("GAME_OPT_ASM_BEGIN");
            InstructionIterator instructions = currentProgram.getListing()
                .getInstructions(function.getBody(), true);
            while (instructions.hasNext()) {
                Instruction instruction = instructions.next();
                out.println(instruction.getAddress() + "\t" + clean(instruction.toString()));
            }
            out.println("GAME_OPT_ASM_END");
        }
    }

    private String clean(String value) {
        if (value == null) {
            return "";
        }
        return value.replace('\t', ' ').replace('\r', ' ').replace('\n', ' ');
    }
}
"#;

/// Ghidra script that lists every function as address, name, and size.
const LIST_FUNCTIONS_SCRIPT: &str = r#"
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;
import ghidra.program.model.listing.FunctionIterator;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;

public class GameOptListFunctions extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        if (args.length < 1) {
            throw new IllegalArgumentException("usage: <output-file>");
        }

        FunctionIterator functions =
            currentProgram.getFunctionManager().getFunctions(true);

        try (PrintWriter out = new PrintWriter(args[0], StandardCharsets.UTF_8)) {
            while (functions.hasNext()) {
                Function function = functions.next();
                long size = function.getBody().getNumAddresses();
                out.println(function.getEntryPoint()
                    + "\t" + clean(function.getName())
                    + "\t" + size);
            }
        }
    }

    private String clean(String value) {
        if (value == null) {
            return "";
        }
        return value.replace('\t', ' ').replace('\r', ' ').replace('\n', ' ');
    }
}
"#;

/// One function handle from [`Decompiler::list_functions`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionSummary {
    pub address: u64,
    pub name: String,
    pub size: u64,
}

/// One decompiled function.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecompiledFunction {
    pub name: String,
    pub address: u64,
    pub pseudocode: String,
    pub assembly: String,
    pub signature: String,
    pub parameters: Vec<String>,
    pub return_type: String,
}

/// Decompiler that uses Ghidra's headless analyzer.
pub struct Decompiler {
    ghidra_path: PathBuf,
    java_home: Option<PathBuf>,
}

impl Decompiler {
    /// Create a new decompiler and validate the surrounding tools.
    pub fn new() -> Result<Self> {
        let ghidra_path = Self::find_ghidra()?;
        info!("Using Ghidra at: {}", ghidra_path.display());

        let java_home = std::env::var_os("JAVA_HOME").map(PathBuf::from);
        if let Some(home) = &java_home {
            if !home.join("bin").join("java").is_file() {
                bail!("JAVA_HOME does not contain bin/java: {}", home.display());
            }
            if let Some(required) = required_java_version(&ghidra_path)
                .and_then(|version| version.parse::<u32>().ok())
            {
                if let Some(actual) = java_major_version(home) {
                    if actual < required {
                        bail!(
                            "JAVA_HOME is Java {}, but the installed Ghidra requires Java {}",
                            actual,
                            required
                        );
                    }
                }
            }
            if !home.join("bin").join("javac").is_file() {
                warn!("JAVA_HOME looks like a JRE (no bin/javac); Ghidra requires a full JDK");
            }
        }

        Ok(Self {
            ghidra_path,
            java_home,
        })
    }

    /// Find a Ghidra installation.
    fn find_ghidra() -> Result<PathBuf> {
        if let Some(dir) = std::env::var_os("GHIDRA_INSTALL_DIR") {
            let path = PathBuf::from(dir);
            if is_ghidra_install(&path) {
                return Ok(path);
            }
            bail!(
                "GHIDRA_INSTALL_DIR is set to {}, but it does not contain \
                 support/analyzeHeadless",
                path.display()
            );
        }

        let mut candidates = vec![
            PathBuf::from("/opt/ghidra"),
            PathBuf::from("/usr/local/ghidra"),
            PathBuf::from("/usr/share/ghidra"),
        ];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("ghidra"));
            candidates.push(home.join(".local").join("share").join("ghidra"));
        }
        #[cfg(windows)]
        candidates.push(PathBuf::from(r"C:\Program Files\Ghidra"));

        for candidate in candidates {
            if is_ghidra_install(&candidate) {
                return Ok(candidate);
            }
        }

        bail!(
            "Ghidra not found. Set GHIDRA_INSTALL_DIR to the installation \
             directory that contains support/analyzeHeadless."
        )
    }

    /// Path to the platform-specific headless launcher.
    fn analyze_headless_path(&self) -> PathBuf {
        let name = if cfg!(windows) {
            "analyzeHeadless.bat"
        } else {
            "analyzeHeadless"
        };
        self.ghidra_path.join("support").join(name)
    }

    /// Decompile the binary's entry point.
    pub async fn decompile_entry(&self, game: &GameInfo) -> Result<DecompiledFunction> {
        let address = entry_point(game)?;
        self.decompile_function(game, address).await
    }

    /// Decompile the function containing `address`.
    pub async fn decompile_function(
        &self,
        game: &GameInfo,
        address: u64,
    ) -> Result<DecompiledFunction> {
        info!("Decompiling 0x{:X} from {}", address, game.path.display());

        let workspace =
            std::env::temp_dir().join(format!("game-opt-ghidra-{}", uuid::Uuid::new_v4()));
        let project_dir = workspace.join("project");
        let script_dir = workspace.join("scripts");
        let output_path = workspace.join("decompiled.txt");
        std::fs::create_dir(&workspace).with_context(|| {
            format!("Failed to create Ghidra workspace {}", workspace.display())
        })?;
        let setup = (|| -> Result<()> {
            std::fs::create_dir(&project_dir).with_context(|| {
                format!(
                    "Failed to create Ghidra project directory {}",
                    project_dir.display()
                )
            })?;
            std::fs::create_dir(&script_dir)?;
            std::fs::write(script_dir.join("GameOptDecompile.java"), DECOMPILE_SCRIPT)
                .context("Failed to write the Ghidra decompile script")?;
            Ok(())
        })();
        if let Err(error) = setup {
            let _ = std::fs::remove_dir_all(&workspace);
            return Err(error);
        }

        let mut command = self.headless_command();
        command
            .arg(&project_dir)
            .arg(GHIDRA_PROJECT_NAME)
            .arg("-import")
            .arg(&game.path)
            .arg("-overwrite")
            .arg("-scriptPath")
            .arg(&script_dir)
            .arg("-postScript")
            .arg("GameOptDecompile.java")
            .arg(format!("0x{:X}", address))
            .arg(&output_path)
            .arg("-deleteProject")
            .current_dir(&workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(java_home) = &self.java_home {
            command.env("JAVA_HOME", java_home);
            prepend_path(&mut command, &java_home.join("bin"));
        }

        let run = tokio::time::timeout(GHIDRA_TIMEOUT, command.output()).await;
        let result = match run {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                self.interpret_output(address, output.status, &stdout, &stderr, &output_path)
            }
            Ok(Err(error)) => Err(error).context("Failed to run Ghidra headless analyzer"),
            Err(_) => Err(anyhow::anyhow!(
                "Ghidra headless timed out after {} seconds",
                GHIDRA_TIMEOUT.as_secs()
            )),
        };

        // The temporary project can be large; never leave it behind.
        let _ = std::fs::remove_dir_all(&workspace);

        result
    }

    /// Build the headless command, wrapping `.bat` files on Windows.
    fn headless_command(&self) -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(self.analyze_headless_path());
            command
        }
        #[cfg(not(windows))]
        {
            Command::new(self.analyze_headless_path())
        }
    }

    fn interpret_output(
        &self,
        address: u64,
        status: std::process::ExitStatus,
        stdout: &str,
        stderr: &str,
        output_path: &Path,
    ) -> Result<DecompiledFunction> {
        let log = tail(stdout, stderr);

        if let Ok(content) = std::fs::read_to_string(output_path) {
            match parse_decompile_output(&content) {
                Ok(function) => {
                    if !status.success() {
                        warn!("Ghidra exited with {} but produced a valid result", status);
                    }
                    return Ok(function);
                }
                Err(error) => {
                    let hint = self.start_hint(&log, "");
                    bail!(
                        "Ghidra output could not be parsed: {:#}{}\nGhidra log tail:\n{}",
                        error,
                        hint,
                        log
                    );
                }
            }
        }

        let hint = self.start_hint(stdout, stderr);
        bail!(
            "Ghidra produced no decompilation result for address 0x{:X}.{}\n{}",
            address,
            hint,
            log
        )
    }

    /// Explain the common Ghidra/JDK startup failures.
    fn start_hint(&self, stdout: &str, stderr: &str) -> String {
        let combined = format!("{}\n{}", stdout, stderr);
        let required = required_java_version(&self.ghidra_path)
            .map(|version| format!(" This Ghidra release requires Java {version}."))
            .unwrap_or_default();

        if combined.contains("unsupported java version")
            || combined.contains("Unable to prompt user for JDK path")
            || combined.contains("Failed to find a supported JDK")
        {
            format!(
                "\nGhidra rejected the configured Java.{required} Set JAVA_HOME to a full JDK \
                 (not a JRE)."
            )
        } else {
            String::new()
        }
    }

    /// Decompile functions requested as comma-separated `0x...` addresses.
    ///
    /// Name-based lookup is intentionally not guessed; an unknown name is
    /// reported and skipped rather than silently decompiling the wrong code.
    pub async fn find_hot_functions(
        &self,
        game_path: &Path,
        specific_functions: Option<&str>,
    ) -> Result<Vec<DecompiledFunction>> {
        info!("Finding hot functions in: {}", game_path.display());

        let Some(specification) = specific_functions else {
            warn!(
                "No specific functions requested; automatic hot-function discovery \
                 is not implemented"
            );
            return Ok(Vec::new());
        };

        let game = GameInfo::from_path(game_path)?;
        let mut functions = Vec::new();

        for requested in specification.split(',') {
            let requested = requested.trim();
            if requested.is_empty() {
                continue;
            }
            if !requested.starts_with("0x") && !requested.starts_with("0X") {
                warn!(
                    "Name-based lookup is not implemented; use a hex address for: {}",
                    requested
                );
                continue;
            }
            let address = parse_hex_address(requested)?;
            functions.push(self.decompile_function(&game, address).await?);
        }

        Ok(functions)
    }
    /// List every function as a cheap `(address, name, size)` handle.
    ///
    /// This runs one Ghidra import and writes no pseudocode. Agents should use
    /// it to choose which functions to decompile, one at a time.
    pub async fn list_functions(&self, game: &GameInfo) -> Result<Vec<FunctionSummary>> {
        info!("Listing functions in {}", game.path.display());

        let workspace =
            std::env::temp_dir().join(format!("game-opt-ghidra-list-{}", uuid::Uuid::new_v4()));
        let project_dir = workspace.join("project");
        let script_dir = workspace.join("scripts");
        let output_path = workspace.join("functions.txt");
        std::fs::create_dir(&workspace).with_context(|| {
            format!("Failed to create Ghidra workspace {}", workspace.display())
        })?;

        let setup = (|| -> Result<()> {
            std::fs::create_dir(&project_dir).with_context(|| {
                format!(
                    "Failed to create Ghidra project directory {}",
                    project_dir.display()
                )
            })?;
            std::fs::create_dir(&script_dir)?;
            std::fs::write(script_dir.join("GameOptListFunctions.java"), LIST_FUNCTIONS_SCRIPT)
                .context("Failed to write the Ghidra function-list script")?;
            Ok(())
        })();
        if let Err(error) = setup {
            let _ = std::fs::remove_dir_all(&workspace);
            return Err(error);
        }

        let mut command = self.headless_command();
        command
            .arg(&project_dir)
            .arg(GHIDRA_PROJECT_NAME)
            .arg("-import")
            .arg(&game.path)
            .arg("-overwrite")
            .arg("-scriptPath")
            .arg(&script_dir)
            .arg("-postScript")
            .arg("GameOptListFunctions.java")
            .arg(&output_path)
            .arg("-deleteProject")
            .current_dir(&workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if let Some(java_home) = &self.java_home {
            command.env("JAVA_HOME", java_home);
            prepend_path(&mut command, &java_home.join("bin"));
        }

        let run = tokio::time::timeout(GHIDRA_TIMEOUT, command.output()).await;
        let result = match run {
            Ok(Ok(output)) => {
                if let Ok(content) = std::fs::read_to_string(&output_path) {
                    parse_function_list(&content)
                } else {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let hint = self.start_hint(&stdout, &stderr);
                    Err(anyhow::anyhow!(
                        "Ghidra produced no function list.{}\n{}",
                        hint,
                        tail(&stdout, &stderr)
                    ))
                }
            }
            Ok(Err(error)) => Err(error).context("Failed to run Ghidra headless analyzer"),
            Err(_) => Err(anyhow::anyhow!(
                "Ghidra headless timed out after {} seconds",
                GHIDRA_TIMEOUT.as_secs()
            )),
        };

        let _ = std::fs::remove_dir_all(&workspace);
        result
    }
}

/// Parse `address\tname\tsize` lines produced by the Ghidra list script.
fn parse_function_list(content: &str) -> Result<Vec<FunctionSummary>> {
    let mut functions = Vec::new();
    for (index, line) in content.lines().enumerate() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let mut fields = line.splitn(3, '\t');
        let address = fields.next().unwrap_or("").trim();
        let name = fields.next().unwrap_or("").trim();
        let size = fields.next().unwrap_or("0").trim();

        let address = address
            .strip_prefix("0x")
            .or_else(|| address.strip_prefix("0X"))
            .unwrap_or(address);
        let address = u64::from_str_radix(address, 16).with_context(|| {
            format!(
                "Invalid function address on line {}: {:?}",
                index + 1,
                address
            )
        })?;
        let size = size.parse::<u64>().unwrap_or(0);
        if name.is_empty() {
            continue;
        }
        functions.push(FunctionSummary {
            address,
            name: name.to_string(),
            size,
        });
    }
    Ok(functions)
}

pub fn is_ghidra_install(path: &Path) -> bool {
    path.join("support").join("analyzeHeadless").is_file()
        || path.join("support").join("analyzeHeadless.bat").is_file()
}

fn prepend_path(command: &mut Command, directory: &Path) {
    if let Some(existing) = std::env::var_os("PATH") {
        let mut paths = vec![directory.to_path_buf()];
        paths.extend(std::env::split_paths(&existing));
        if let Ok(joined) = std::env::join_paths(paths) {
            command.env("PATH", joined);
            return;
        }
    }
    command.env("PATH", directory);
}

/// Determine the entry point from the binary itself.
pub fn entry_point(game: &GameInfo) -> Result<u64> {
    let data = std::fs::read(&game.path)
        .with_context(|| format!("Failed to read {}", game.path.display()))?;
    let object = goblin::Object::parse(&data)
        .with_context(|| format!("Failed to parse {}", game.path.display()))?;

    match object {
        goblin::Object::PE(pe) => (pe.image_base as u64)
            .checked_add(pe.entry as u64)
            .context("PE entry point overflowed"),
        goblin::Object::Elf(elf) => Ok(elf.entry),
        goblin::Object::Mach(goblin::mach::Mach::Binary(macho)) => Ok(macho.entry),
        goblin::Object::Mach(_) => {
            bail!("Fat Mach-O binaries have no single entry point; pass --address")
        }
        _ => bail!(
            "Cannot determine an entry point for {}; pass --address",
            game.path.display()
        ),
    }
}

/// Parse `0x1234` or bare hexadecimal into an address.
pub fn parse_hex_address(value: &str) -> Result<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("Address is empty; expected hexadecimal, e.g. 0x00401000");
    }
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if hex.is_empty() {
        bail!("Address is empty; expected hexadecimal, e.g. 0x00401000");
    }
    u64::from_str_radix(hex, 16).with_context(|| format!("Invalid hexadecimal address: {}", value))
}

/// Required Java major version from Ghidra's application.properties.
pub fn required_java_version(ghidra_dir: &Path) -> Option<String> {
    let properties =
        std::fs::read_to_string(ghidra_dir.join("Ghidra").join("application.properties")).ok()?;
    properties
        .lines()
        .find_map(|line| line.strip_prefix("application.java.min="))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Read the Java major version from a JDK's `release` file.
///
/// Returns `None` when the file or field is missing, or the value is not a
/// recognizable version. It does not execute Java.
pub fn java_major_version(java_home: &Path) -> Option<u32> {
    let release = std::fs::read_to_string(java_home.join("release")).ok()?;
    let version = release
        .lines()
        .find_map(|line| line.strip_prefix("JAVA_VERSION="))?
        .trim()
        .trim_matches('"');
    let first = version.split(['.', '-', '+']).next()?;
    if first == "1" {
        version.split('.').nth(1)?.parse().ok()
    } else {
        first.parse().ok()
    }
}

fn parse_decompile_output(content: &str) -> Result<DecompiledFunction> {
    let mut name: Option<String> = None;
    let mut address: Option<u64> = None;
    let mut signature = String::new();
    let mut return_type = String::new();
    let mut parameters = Vec::new();
    let mut pseudocode = String::new();
    let mut assembly_lines = Vec::new();
    let mut in_pseudocode = false;
    let mut in_assembly = false;
    let mut saw_pseudocode = false;
    let mut saw_assembly = false;

    for line in content.lines() {
        if in_pseudocode {
            if line == "GAME_OPT_PSEUDO_END" {
                in_pseudocode = false;
            } else {
                pseudocode.push_str(line);
                pseudocode.push('\n');
            }
            continue;
        }
        if in_assembly {
            if line == "GAME_OPT_ASM_END" {
                in_assembly = false;
            } else if let Some((instruction_address, text)) = line.split_once('\t') {
                assembly_lines.push(format!("{}: {}", instruction_address, text));
            } else if !line.trim().is_empty() {
                assembly_lines.push(line.to_string());
            }
            continue;
        }

        if line == "GAME_OPT_PSEUDO_BEGIN" {
            saw_pseudocode = true;
            in_pseudocode = true;
        } else if line == "GAME_OPT_ASM_BEGIN" {
            saw_assembly = true;
            in_assembly = true;
        } else if let Some(value) = line.strip_prefix("GAME_OPT_NAME\t") {
            name = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("GAME_OPT_ENTRY\t") {
            address = Some(parse_hex_address(value)?);
        } else if let Some(value) = line.strip_prefix("GAME_OPT_SIGNATURE\t") {
            signature = value.to_string();
        } else if let Some(value) = line.strip_prefix("GAME_OPT_RETURN\t") {
            return_type = value.to_string();
        } else if let Some(value) = line.strip_prefix("GAME_OPT_PARAM\t") {
            parameters.push(value.to_string());
        }
    }

    if in_pseudocode {
        bail!("Ghidra output ended before the pseudocode end marker");
    }
    if in_assembly {
        bail!("Ghidra output ended before the assembly end marker");
    }
    if !saw_pseudocode {
        bail!("Ghidra output has no pseudocode section");
    }
    if !saw_assembly {
        bail!("Ghidra output has no assembly section");
    }

    let name = name.context("Ghidra output is missing the function name")?;
    if name.trim().is_empty() {
        bail!("Ghidra returned an empty function name");
    }
    let address = address.context("Ghidra output is missing the function entry address")?;
    let pseudocode = pseudocode.trim().to_string();
    if pseudocode.is_empty() {
        bail!("Ghidra returned empty pseudocode for {}", name);
    }

    Ok(DecompiledFunction {
        name,
        address,
        pseudocode,
        assembly: assembly_lines.join("\n"),
        signature,
        parameters,
        return_type,
    })
}

/// Last part of Ghidra's log, safe for multi-byte UTF-8.
fn tail(stdout: &str, stderr: &str) -> String {
    let combined = format!("{}\n{}", stdout.trim_end(), stderr.trim_end());
    let lines: Vec<&str> = combined.lines().collect();
    let start = lines.len().saturating_sub(40);
    lines[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
GAME_OPT_NAME\tentry
GAME_OPT_ENTRY\t109214b6
GAME_OPT_SIGNATURE\tundefined entry(void)
GAME_OPT_RETURN\tundefined
GAME_OPT_PARAM\tint argc
GAME_OPT_PARAM\tchar ** argv
GAME_OPT_PSEUDO_BEGIN

void entry(void)
{
  return;
}
GAME_OPT_PSEUDO_END
GAME_OPT_ASM_BEGIN
109214b6\tPUSH EBP
109214b7\tMOV EBP,ESP
GAME_OPT_ASM_END
";

    #[test]
    fn parses_a_complete_ghidra_result() {
        let function = parse_decompile_output(SAMPLE).unwrap();

        assert_eq!(function.name, "entry");
        assert_eq!(function.address, 0x1092_14b6);
        assert_eq!(function.signature, "undefined entry(void)");
        assert_eq!(function.return_type, "undefined");
        assert_eq!(function.parameters, vec!["int argc", "char ** argv"]);
        assert!(function.pseudocode.contains("void entry(void)"));
        assert!(!function.pseudocode.starts_with('\n'));
        assert_eq!(
            function.assembly,
            "109214b6: PUSH EBP\n109214b7: MOV EBP,ESP"
        );
    }

    #[test]
    fn rejects_output_without_pseudocode() {
        let error = parse_decompile_output("GAME_OPT_NAME\tentry\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no pseudocode section"), "error: {}", error);
    }

    #[test]
    fn rejects_empty_pseudocode() {
        let content = "\
GAME_OPT_NAME\tentry
GAME_OPT_ENTRY\t109214b6
GAME_OPT_PSEUDO_BEGIN

GAME_OPT_PSEUDO_END
GAME_OPT_ASM_BEGIN
GAME_OPT_ASM_END
";
        let error = parse_decompile_output(content).unwrap_err().to_string();
        assert!(error.contains("empty pseudocode"), "error: {}", error);
    }

    #[test]
    fn rejects_output_without_an_assembly_section() {
        let content = "\
GAME_OPT_NAME\tentry
GAME_OPT_ENTRY\t109214b6
GAME_OPT_PSEUDO_BEGIN
void entry(void) {}
GAME_OPT_PSEUDO_END
";
        let error = parse_decompile_output(content).unwrap_err().to_string();
        assert!(error.contains("no assembly section"), "error: {}", error);
    }

    #[test]
    fn rejects_an_empty_function_name() {
        let content = "\
GAME_OPT_NAME\t
GAME_OPT_ENTRY\t109214b6
GAME_OPT_PSEUDO_BEGIN
void entry(void) {}
GAME_OPT_PSEUDO_END
GAME_OPT_ASM_BEGIN
GAME_OPT_ASM_END
";
        let error = parse_decompile_output(content).unwrap_err().to_string();
        assert!(error.contains("empty function name"), "error: {}", error);
    }

    #[test]
    fn rejects_an_unterminated_pseudocode_section() {
        let content = "\
GAME_OPT_NAME\tentry
GAME_OPT_ENTRY\t109214b6
GAME_OPT_PSEUDO_BEGIN
void entry(void) {}
";
        let error = parse_decompile_output(content).unwrap_err().to_string();
        assert!(error.contains("end marker"), "error: {}", error);
    }

    #[test]
    fn rejects_an_unterminated_assembly_section() {
        let content = "\
GAME_OPT_NAME\tentry
GAME_OPT_ENTRY\t109214b6
GAME_OPT_PSEUDO_BEGIN
void entry(void) {}
GAME_OPT_PSEUDO_END
GAME_OPT_ASM_BEGIN
109214b6\tPUSH EBP
";
        let error = parse_decompile_output(content).unwrap_err().to_string();
        assert!(error.contains("assembly end marker"), "error: {}", error);
    }

    #[test]
    fn parses_hex_addresses_with_and_without_prefix() {
        assert_eq!(parse_hex_address("0x109214b6").unwrap(), 0x1092_14b6);
        assert_eq!(parse_hex_address("109214B6").unwrap(), 0x1092_14b6);
        assert_eq!(parse_hex_address(" 0X10 ").unwrap(), 0x10);
    }

    #[test]
    fn rejects_invalid_addresses() {
        assert!(parse_hex_address("").is_err());
        assert!(parse_hex_address("0x").is_err());
        assert!(parse_hex_address("hello").is_err());
        assert!(parse_hex_address("0x1g").is_err());
    }

    #[test]
    fn reads_ghidra_java_requirement() {
        let dir = tempfile::tempdir().unwrap();
        let ghidra = dir.path().join("Ghidra");
        std::fs::create_dir_all(&ghidra).unwrap();
        std::fs::write(
            ghidra.join("application.properties"),
            "application.name=Ghidra\napplication.java.min=25\n",
        )
        .unwrap();

        assert_eq!(required_java_version(dir.path()).as_deref(), Some("25"));
    }

    #[test]
    fn missing_ghidra_properties_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(required_java_version(dir.path()), None);
    }

    #[test]
    fn reads_java_major_version_from_a_jdk_release_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("release"),
            "IMPLEMENTOR=\"Eclipse Adoptium\"\nJAVA_VERSION=\"25.0.5\"\n",
        )
        .unwrap();
        assert_eq!(java_major_version(dir.path()), Some(25));
    }

    #[test]
    fn reads_legacy_java_version_format() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("release"), "JAVA_VERSION=\"1.8.0_402\"\n").unwrap();
        assert_eq!(java_major_version(dir.path()), Some(8));
    }

    #[test]
    fn missing_java_release_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(java_major_version(dir.path()), None);
    }

    #[test]
    fn detects_a_ghidra_installation_by_its_launcher() {
        let dir = tempfile::tempdir().unwrap();
        let support = dir.path().join("support");
        std::fs::create_dir_all(&support).unwrap();
        assert!(!is_ghidra_install(dir.path()));

        std::fs::write(support.join("analyzeHeadless"), "#!/bin/sh\n").unwrap();
        assert!(is_ghidra_install(dir.path()));
    }

    #[test]
    fn parses_a_function_list_with_addresses_and_sizes() {
        let functions =
            parse_function_list("109214b6\tentry\t42\n10a00000\tFUN_10a00000\t128\n\n").unwrap();
        assert_eq!(functions.len(), 2);
        assert_eq!(
            functions[0],
            FunctionSummary {
                address: 0x109214b6,
                name: "entry".to_string(),
                size: 42,
            }
        );
        assert_eq!(functions[1].address, 0x10a00000);
        assert_eq!(functions[1].size, 128);
    }

    #[test]
    fn rejects_a_malformed_function_address() {
        let error = parse_function_list("not-hex\tentry\t42\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("Invalid function address"), "error: {error}");
    }
}
