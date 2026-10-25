//! Encrypted artifact transfers. Destination/engine/transport remain separate.
pub mod artifact;
pub mod coordinator;
pub mod local_artifact;
pub mod local_backend;
pub mod model;
pub mod pc_backup;
pub mod reverse_tunnel;
pub mod sftp_pull;
pub mod ssh_stream;
pub use coordinator::Coordinator;
pub use model::*;
