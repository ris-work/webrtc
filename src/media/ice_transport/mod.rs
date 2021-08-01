#[cfg(test)]
mod ice_transport_test;

pub mod ice_transport_state;

use crate::media::ice_transport::ice_transport_state::ICETransportState;
use crate::peer::ice::ice_candidate::ice_candidate_pair::ICECandidatePair;
use crate::peer::ice::ice_gather::ice_gatherer::ICEGatherer;
use crate::peer::ice::ice_role::ICERole;
use crate::util::mux::{Config, Mux};

use crate::error::Error;
use crate::peer::ice::ice_candidate::ICECandidate;
use crate::peer::ice::ICEParameters;
use crate::util::mux::endpoint::Endpoint;
use crate::util::mux::mux_func::MatchFunc;
use crate::RECEIVE_MTU;

use ice::candidate::Candidate;
use ice::state::ConnectionState;

use anyhow::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use util::Conn;

pub type OnConnectionStateChangeHdlrFn = Box<
    dyn (FnMut(ICETransportState) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnSelectedCandidatePairChangeHdlrFn = Box<
    dyn (FnMut(ICECandidatePair) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

#[derive(Default)]
pub struct ICETransportInternal {
    role: ICERole,
    conn: Option<Arc<dyn Conn + Send + Sync>>, //AgentConn
    mux: Option<Mux>,
    cancel_tx: Option<mpsc::Sender<()>>,
}

/// ICETransport allows an application access to information about the ICE
/// transport over which packets are sent and received.
#[derive(Default)]
pub struct ICETransport {
    gatherer: Arc<ICEGatherer>,
    on_connection_state_change_handler: Arc<Mutex<Option<OnConnectionStateChangeHdlrFn>>>,
    on_selected_candidate_pair_change_handler:
        Arc<Mutex<Option<OnSelectedCandidatePairChangeHdlrFn>>>,
    state: Arc<AtomicU8>, // ICETransportState
    internal: Mutex<ICETransportInternal>,
}

impl ICETransport {
    /// creates a new new_icetransport.
    pub(crate) fn new(gatherer: Arc<ICEGatherer>) -> Self {
        ICETransport {
            state: Arc::new(AtomicU8::new(ICETransportState::New as u8)),
            gatherer,
            ..Default::default()
        }
    }

    /// get_selected_candidate_pair returns the selected candidate pair on which packets are sent
    /// if there is no selected pair nil is returned
    pub async fn get_selected_candidate_pair(&self) -> Option<ICECandidatePair> {
        if let Some(agent) = self.gatherer.get_agent().await {
            if let Some(ice_pair) = agent.get_selected_candidate_pair().await {
                let local = ICECandidate::from(&ice_pair.local);
                let remote = ICECandidate::from(&ice_pair.remote);
                return Some(ICECandidatePair::new(local, remote));
            }
        }
        None
    }

    /// Start incoming connectivity checks based on its configured role.
    pub async fn start(
        &self,
        //gatherer: Option<ICEGatherer>,
        params: ICEParameters,
        role: Option<ICERole>,
    ) -> Result<()> {
        if self.state() != ICETransportState::New {
            return Err(Error::ErrICETransportNotInNew.into());
        }

        self.ensure_gatherer().await?;

        if let Some(agent) = self.gatherer.get_agent().await {
            let state = Arc::clone(&self.state);

            let on_connection_state_change_handler =
                Arc::clone(&self.on_connection_state_change_handler);
            agent
                .on_connection_state_change(Box::new(move |ice_state: ConnectionState| {
                    let s = ICETransportState::from(ice_state);
                    let on_connection_state_change_handler_clone =
                        Arc::clone(&on_connection_state_change_handler);
                    state.store(s as u8, Ordering::SeqCst);
                    Box::pin(async move {
                        let mut handler = on_connection_state_change_handler_clone.lock().await;
                        if let Some(f) = &mut *handler {
                            f(s).await;
                        }
                    })
                }))
                .await;

            let on_selected_candidate_pair_change_handler =
                Arc::clone(&self.on_selected_candidate_pair_change_handler);
            agent
                .on_selected_candidate_pair_change(Box::new(
                    move |local: &Arc<dyn Candidate + Send + Sync>,
                          remote: &Arc<dyn Candidate + Send + Sync>| {
                        let on_selected_candidate_pair_change_handler_clone =
                            Arc::clone(&on_selected_candidate_pair_change_handler);
                        let local = ICECandidate::from(local);
                        let remote = ICECandidate::from(remote);
                        Box::pin(async move {
                            let mut handler =
                                on_selected_candidate_pair_change_handler_clone.lock().await;
                            if let Some(f) = &mut *handler {
                                f(ICECandidatePair::new(local, remote)).await;
                            }
                        })
                    },
                ))
                .await;

            let role = if let Some(role) = role {
                role
            } else {
                ICERole::Controlled
            };

            let (cancel_tx, cancel_rx) = mpsc::channel(1);

            let conn: Arc<dyn Conn + Send + Sync> = match role {
                ICERole::Controlling => {
                    agent
                        .dial(
                            cancel_rx,
                            params.username_fragment.clone(),
                            params.password.clone(),
                        )
                        .await?
                }

                ICERole::Controlled => {
                    agent
                        .accept(
                            cancel_rx,
                            params.username_fragment.clone(),
                            params.password.clone(),
                        )
                        .await?
                }

                _ => return Err(Error::ErrICERoleUnknown.into()),
            };

            let config = Config {
                conn: Arc::clone(&conn),
                buffer_size: RECEIVE_MTU,
            };

            {
                let mut internal = self.internal.lock().await;
                internal.role = role;
                internal.cancel_tx = Some(cancel_tx);
                internal.conn = Some(conn);
                internal.mux = Some(Mux::new(config));
            }

            Ok(())
        } else {
            Err(Error::ErrICEAgentNotExist.into())
        }
    }

    /// restart is not exposed currently because ORTC has users create a whole new ICETransport
    /// so for now lets keep it private so we don't cause ORTC users to depend on non-standard APIs
    pub(crate) async fn restart(&mut self) -> Result<()> {
        if let Some(agent) = self.gatherer.get_agent().await {
            agent
                .restart(
                    self.gatherer
                        .setting_engine
                        .candidates
                        .username_fragment
                        .clone(),
                    self.gatherer.setting_engine.candidates.password.clone(),
                )
                .await?;
        } else {
            return Err(Error::ErrICEAgentNotExist.into());
        }
        self.gatherer.gather().await
    }

    /// Stop irreversibly stops the ICETransport.
    pub async fn stop(&self) -> Result<()> {
        self.set_state(ICETransportState::Closed);

        {
            let mut internal = self.internal.lock().await;
            internal.cancel_tx.take();
            if let Some(mut mux) = internal.mux.take() {
                mux.close().await;
            }
        }

        self.gatherer.close().await?;

        Ok(())
    }

    /// on_selected_candidate_pair_change sets a handler that is invoked when a new
    /// ICE candidate pair is selected
    pub async fn on_selected_candidate_pair_change(&self, f: OnSelectedCandidatePairChangeHdlrFn) {
        let mut on_selected_candidate_pair_change_handler =
            self.on_selected_candidate_pair_change_handler.lock().await;
        *on_selected_candidate_pair_change_handler = Some(f);
    }

    /// on_connection_state_change sets a handler that is fired when the ICE
    /// connection state changes.
    pub async fn on_connection_state_change(&self, f: OnConnectionStateChangeHdlrFn) {
        let mut on_connection_state_change_handler =
            self.on_connection_state_change_handler.lock().await;
        *on_connection_state_change_handler = Some(f);
    }

    /// Role indicates the current role of the ICE transport.
    pub async fn role(&self) -> ICERole {
        let internal = self.internal.lock().await;
        internal.role
    }

    /// set_remote_candidates sets the sequence of candidates associated with the remote ICETransport.
    pub async fn set_remote_candidates(
        &mut self,
        remote_candidates: &[ICECandidate],
    ) -> Result<()> {
        self.ensure_gatherer().await?;

        if let Some(agent) = self.gatherer.get_agent().await {
            for rc in remote_candidates {
                let c: Arc<dyn Candidate + Send + Sync> = Arc::new(rc.to_ice().await?);
                agent.add_remote_candidate(&c).await?;
            }
            Ok(())
        } else {
            Err(Error::ErrICEAgentNotExist.into())
        }
    }

    /// adds a candidate associated with the remote ICETransport.
    pub async fn add_remote_candidate(
        &mut self,
        remote_candidate: Option<ICECandidate>,
    ) -> Result<()> {
        self.ensure_gatherer().await?;

        if let Some(agent) = self.gatherer.get_agent().await {
            if let Some(r) = remote_candidate {
                let c: Arc<dyn Candidate + Send + Sync> = Arc::new(r.to_ice().await?);
                agent.add_remote_candidate(&c).await?;
            }

            Ok(())
        } else {
            Err(Error::ErrICEAgentNotExist.into())
        }
    }

    /// State returns the current ice transport state.
    pub fn state(&self) -> ICETransportState {
        ICETransportState::from(self.state.load(Ordering::SeqCst))
    }

    pub(crate) fn set_state(&self, s: ICETransportState) {
        self.state.store(s as u8, Ordering::SeqCst)
    }

    pub(crate) async fn new_endpoint(&self, f: MatchFunc) -> Option<Arc<Endpoint>> {
        let internal = self.internal.lock().await;
        if let Some(mux) = &internal.mux {
            Some(mux.new_endpoint(f).await)
        } else {
            None
        }
    }

    pub(crate) async fn ensure_gatherer(&self) -> Result<()> {
        if self.gatherer.get_agent().await.is_none() {
            self.gatherer.create_agent().await
        } else {
            Ok(())
        }
    }

    /*TODO:
    func (t *ICETransport) collectStats(collector *statsReportCollector) {
        t.lock.Lock()
        conn := t.conn
        t.lock.Unlock()

        collector.Collecting()

        stats := TransportStats{
            Timestamp: statsTimestampFrom(time.Now()),
            Type:      StatsTypeTransport,
            ID:        "ice_transport",
        }

        if conn != nil {
            stats.BytesSent = conn.BytesSent()
            stats.BytesReceived = conn.BytesReceived()
        }

        collector.Collect(stats.ID, stats)
    }
    */

    pub(crate) async fn have_remote_credentials_change(
        &self,
        new_ufrag: String,
        new_pwd: String,
    ) -> bool {
        if let Some(agent) = self.gatherer.get_agent().await {
            let (ufrag, upwd) = agent.get_remote_user_credentials().await;
            ufrag != new_ufrag || upwd != new_pwd
        } else {
            false
        }
    }

    pub(crate) async fn set_remote_credentials(
        &self,
        new_ufrag: String,
        new_pwd: String,
    ) -> Result<()> {
        if let Some(agent) = self.gatherer.get_agent().await {
            Ok(agent.set_remote_credentials(new_ufrag, new_pwd).await?)
        } else {
            Err(Error::ErrICEAgentNotExist.into())
        }
    }
}
