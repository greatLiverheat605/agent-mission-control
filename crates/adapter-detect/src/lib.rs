mod path_probe;
mod version;

pub use adapter_core::InstallState;
pub use path_probe::{
    ProbeError, ProbeOptions, VersionProbe, probe_executable, resolve_executable,
};
pub use version::{AgentKind, Detection, detect, detect_all};
