"""
REA-style AI agent interface for Game Opt Tool.

This mirrors how REA works: an MCP server that AI agents call
to reverse engineer binaries. Our tool exposes the same capability
pattern but via Rust + Ghidra.
"""

import json
import subprocess
import sys
from typing import Optional

class REAAgent:
    """Mirrors REA's agent interface for game binary reverse engineering."""
    
    def __init__(self, tool_path: str = "./target/release/game-opt-tool"):
        self.tool_path = tool_path
        self.mcp_endpoint = "http://localhost:9000/mcp"
        
    def _call_mcp(self, method: str, params: dict) -> dict:
        """Call the MCP server (our tool) with a request."""
        try:
            import requests
            response = requests.post(
                self.mcp_endpoint,
                json={"method": method, "params": params},
                timeout=120
            )
            if response.status_code == 200:
                return response.json()
            return {"error": f"MCP request failed: {response.status_code}"}
        except Exception as e:
            return {"error": str(e)}
    
    def decompile(self, address: str, game_path: str) -> dict:
        """Decompile a function at the given address."""
        result = self._call_mcp("decompile", {
            "address": address,
            "game_path": game_path
        })
        # Mirror REA's response format
        if "error" not in result:
            return {
                "success": True,
                "pseudocode": result.get("pseudocode", ""),
                "assembly": result.get("assembly", []),
                "name": result.get("name", "unknown_function")
            }
        return {"success": False, "error": result.get("error", "Unknown error")}
    
    def analyze(self, code: str, game_engine: str = "unknown") -> dict:
        """Analyze decompiled code for optimization opportunities."""
        result = self._call_mcp("analyze", {
            "code": code,
            "game_engine": game_engine
        })
        
        if "error" not in result:
            return {
                "success": True,
                "has_while_loops": result.get("has_while_loops", False),
                "has_for_loops": result.get("has_for_loops", False),
                "code_length": result.get("code_length", 0),
                "suggestions": result.get("suggestions", []),
                "game_engine": result.get("game_engine", game_engine)
            }
        return {"success": False, "error": result.get("error", "Unknown error")}
    
    def patch(self, game_path: str, output_path: str, spec_path: str) -> dict:
        """Apply a patch spec to a binary."""
        result = self._call_mcp("patch", {
            "game_path": game_path,
            "output_path": output_path,
            "spec_path": spec_path
        })
        
        if "success" in result and result.get("success"):
            return {
                "success": True,
                "patches_applied": result.get("patches_applied", 0),
                "sections_applied": result.get("sections_applied", 0),
                "manifest_applied": result.get("manifest_applied", False),
                "output_sha256": result.get("output_sha256", "")
            }
        return {"success": False, "error": result.get("error", "Unknown error")}
    
    def verify(self, original_path: str, patched_path: str, manifest_path: str) -> dict:
        """Verify a patch by comparing original and patched binary."""
        result = self._call_mcp("verify", {
            "original_path": original_path,
            "patched_path": patched_path,
            "manifest_path": manifest_path
        })
        
        if "success" in result and result.get("success"):
            return {
                "success": True,
                "patches_reverted": result.get("patches_reverted", 0),
                "sections_removed": result.get("sections_removed", 0),
                "output_sha256": result.get("output_sha256", ""),
                "restored_sha256": result.get("restored_sha256", "")
            }
        return {"success": False, "error": result.get("error", "Unknown error")}


# Example usage mirroring REA pattern
if __name__ == "__main__":
    agent = REAAgent()
    
    # Example: Decompile the Deus Ex entry point
    result = agent.decompile("0x109214b6", "/path/to/DeusEx.exe")
    if result["success"]:
        print(f"Decompiled function: {result['name']}")
        print(f"Pseudocode:\\n{result['pseudocode']}")
        print(f"Assembly:\\n{'\\n'.join(result['assembly'])}")
    else:
        print(f"Decompile failed: {result['error']}")
EOF
echo "REA-style agent created"
