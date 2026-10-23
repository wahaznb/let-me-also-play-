//! Deus Ex 2000 target module
//!
//! Provides game-specific functionality for optimizing Deus Ex

pub mod config;
pub mod profiler;
pub mod decompiler;
pub mod optimizer;
pub mod patcher;
pub mod verifier;

pub use config::DeusExInfo;
pub use profiler::DeusExProfiler;
pub use decompiler::DeusExDecompiler;
pub use optimizer::DeusExOptimizer;
pub use patcher::DeusExPatcher;
pub use verifier::DeusExVerifier;
