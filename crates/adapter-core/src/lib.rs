pub mod adapter;
pub mod capability;
pub mod fake;

pub use adapter::{
    AdapterError, AgentAdapter, AgentControl, AgentEvent, AgentHandle, EventSink, LoadoutSnapshot,
    StartAgentRequest,
};
pub use capability::{
    AgentCapabilityReport, Capability, InstallState, ProviderCapability, ProviderId,
};
pub use fake::FakeAdapter;
