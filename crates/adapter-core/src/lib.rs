pub mod adapter;
pub mod capability;
pub mod fake;

pub use adapter::{
    AdapterError, AgentAdapter, AgentEvent, AgentHandle, EventSink, StartAgentRequest,
};
pub use capability::{AgentCapabilityReport, Capability, InstallState};
pub use fake::FakeAdapter;
