use super::candidate_base::*;
use super::*;
use crate::rand::generate_cand_id;
use std::sync::atomic::{AtomicU16, AtomicU8};
use std::sync::Arc;

// CandidateHostConfig is the config required to create a new CandidateHost
#[derive(Default)]
pub struct CandidateHostConfig {
    pub base_config: CandidateBaseConfig,

    pub tcp_type: TCPType,
}

impl CandidateHostConfig {
    // NewCandidateHost creates a new host candidate
    pub async fn new_candidate_host(
        self,
        agent_internal: Arc<Mutex<AgentInternal>>,
    ) -> Result<CandidateBase, Error> {
        let mut candidate_id = self.base_config.candidate_id;
        if candidate_id.is_empty() {
            candidate_id = generate_cand_id();
        }

        let mut c = CandidateBase {
            id: candidate_id,
            address: self.base_config.address.clone(),
            candidate_type: CandidateType::Host,
            component: Arc::new(AtomicU16::new(self.base_config.component)),
            port: self.base_config.port,
            tcp_type: self.tcp_type,
            foundation_override: self.base_config.foundation,
            priority_override: self.base_config.priority,
            network: self.base_config.network,
            network_type: Arc::new(AtomicU8::new(NetworkType::UDP4 as u8)),
            conn: self.base_config.conn,
            agent_internal: Some(agent_internal),
            ..Default::default()
        };

        if !self.base_config.address.ends_with(".local") {
            let ip = self.base_config.address.parse()?;
            c.set_ip(&ip)?;
        };

        Ok(c)
    }
}
