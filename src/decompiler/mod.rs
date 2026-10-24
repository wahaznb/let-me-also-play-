//! Ghidra integration for decompilation
//!
//! Uses Ghidra's headless mode to decompile game binaries

use crate::game::GameInfo;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn, error};

/// Decompiled function
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

/// Decompiler that uses Ghidra
pub struct Decompiler {
    ghidra_path: PathBuf,
    java_home: Option<PathBuf>,
}

impl Decompiler {
    /// Create a new decompiler
    pub fn new() -> Result<Self> {
        // Try to find Ghidra
        let ghidra_path = Self::find_ghidra()?;

        info!("Using Ghidra at: {:?}", ghidra_path);

        Ok(Self {
            ghidra_path,
            java_home: std::env::var("JAVA_HOME").ok().map(PathBuf::from),
        })
    }

    /// Find Ghidra installation
    fn find_ghidra() -> Result<PathBuf> {
        // Check environment variable first
        if let Ok(path) = std::env::var("GHIDRA_INSTALL_DIR") {
            let p = PathBuf::from(path);
            if p.join("support").join("analyze").exists() {
                return Ok(p);
            }
        }

        // Check common installation locations
        let candidates = vec![
            "/opt/ghidra",
            "/usr/local/ghidra",
            "~/ghidra",
            "C:\\Program Files\\Ghidra",
        ];

        for candidate in candidates {
            let expanded = candidate.replace("~", dirs::home_dir().unwrap_or_default().to_string_lossy().as_ref());
            let p = PathBuf::from(expanded);
            if p.join("support").join("analyze").exists() {
                return Ok(p);
            }
        }

        Err(anyhow::anyhow!(
            "Ghidra not found. Please install Ghidra and set GHIDRA_INSTALL_DIR environment variable."
        ))
    }

    /// Get the Ghidra script path
    fn ghidra_scripts_path(&self) -> PathBuf {
        self.ghidra_path.join("support").join("analyze")
    }

    /// Decompile a specific function
    pub async fn decompile_function(
        &self,
        game_info: &GameInfo,
        function_address: u64,
    ) -> Result<DecompiledFunction> {
        info!("Decompiling function at 0x{:X}", function_address);

        // Create a temporary Ghidra project
        let project_dir = std::env::temp_dir().join(format!("game-opt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&project_dir)?;

        // Run Ghidra headless analyzer
        let analyze_script = self.ghidra_scripts_path().join("analyzeHeadless");
        
        let output = Command::new(&analyze_script)
            .arg(&project_dir)
            .arg("true")
            .arg("-import")
            .arg(&game_info.path)
            .arg("-projectName")
            .arg("temp")
            .arg("-scriptPath")
            .arg(self.ghidra_scripts_path().join("auto_analysis"))
            .arg("-postScript")
            .arg(self.create_decompile_script(function_address)?)
            .output()
            .context("Failed to run Ghidra headless analyzer")?;

        if !output.status.success() {
            error!("Ghidra analysis failed: {}", String::from_utf8_lossy(&output.stderr));
        }

        // Parse the output to extract decompiled code
        // This is simplified - a real implementation would parse Ghidra's XML/JSON output
        let pseudocode = self.parse_decompile_output(&project_dir, function_address).await?;

        Ok(DecompiledFunction {
            name: format!("func_0x{:X}", function_address),
            address: function_address,
            pseudocode,
            assembly: String::new(), // Would come from disassembly
            signature: String::new(),
            parameters: vec![],
            return_type: String::new(),
        })
    }

    /// Create a Ghidra script for decompilation
    fn create_decompile_script(
        &self,
        function_address: u64,
    ) -> Result<PathBuf> {
        let script_path = std::env::temp_dir().join(format!("decompile_0x{:X}.py", function_address));
        
        let script_content = format!(
            r#"
from ghidra.program.model.listing import Function
from ghidra.util.task import ConsoleTaskMonitor

# Get the current program
program = currentProgram

# Find the function at the specified address
addr = program.getAddressFactory().getAddress("0x{:X}")
functions = program.getFunctionManager().getFunctions(addr, True)

if functions.hasNext():
    func = functions.next()
    println("Found function: " + func.getName())
    
    # Get the decompiler
    decompiler = tool.getService(class com.github.javaparser.JavaParser)
    if decompiler:
        code = decompiler.decompile(func, 0)
        println("=== DECOMPILATION ===")
        println(code.getDecompiledFunction().toString())
        println("=== END DECOMPILATION ===")
else:
    println("Function not found at address 0x{:X}", addr)
"#,
            function_address, function_address
        );

        std::fs::write(&script_path, script_content)?;
        Ok(script_path)
    }

    /// Parse decompile output
    async fn parse_decompile_output(
        &self,
        project_dir: &Path,
        _function_address: u64,
    ) -> Result<String> {
        // Look for decompile output files
        let output_dirs = vec![
            project_dir.join("temp").join("logs"),
            project_dir.join("temp").join("scripts"),
        ];

        for dir in &output_dirs {
            if dir.exists() {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map(|e| e == "log") == Some(true) {
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                if content.contains("DECOMPILATION") {
                                    return Ok(content);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Return placeholder if no output found
        Ok("// Decompiled code would be here\n// This requires Ghidra to be properly configured"
            .to_string())
    }

    /// Find hot functions using Ghidra
    pub async fn find_hot_functions(
        &self,
        game_path: &Path,
        specific_functions: Option<&str>,
    ) -> Result<Vec<DecompiledFunction>> {
        info!("Finding hot functions in: {:?}", game_path);

        let mut functions = Vec::new();

        // If specific functions are requested, decompile those
        if let Some(funcs_str) = specific_functions {
            for func in funcs_str.split(',') {
                let func = func.trim();
                if func.starts_with('0') && func.contains('x') {
                    // Address format (0x1234)
                    let addr = u64::from_str_radix(&func[2..], 16)?;
                    let decompiled = self.decompile_function(
                        &GameInfo::from_path(game_path)?,
                        addr,
                    ).await?;
                    functions.push(decompiled);
                } else {
                    // Name format - search in binary
                    warn!("Name-based lookup not yet implemented for: {}", func);
                }
            }
        } else {
            // Use config to find known hot functions
            // This would be expanded based on the detected game
            warn!("No specific functions requested; automatic hot-function discovery is not implemented");
        }

        Ok(functions)
    }
}
