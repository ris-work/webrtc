#[cfg(test)]
pub(crate) mod peer_connection_test;

mod peer_connection_internal;

use crate::api::media_engine::MediaEngine;
use crate::api::setting_engine::SettingEngine;
use crate::api::API;
use crate::data::data_channel::DataChannel;
use crate::data::sctp_transport::SCTPTransport;
use crate::media::dtls_transport::dtls_transport_state::DTLSTransportState;
use crate::media::dtls_transport::DTLSTransport;
use crate::media::ice_transport::ice_transport_state::ICETransportState;
use crate::media::ice_transport::ICETransport;
use crate::media::interceptor::{Attributes, Interceptor, RTCPWriter};
use crate::media::rtp::rtp_receiver::RTPReceiver;
use crate::media::rtp::rtp_transceiver::{
    find_by_mid, handle_unknown_rtp_packet, satisfy_type_and_direction, RTPTransceiver,
};
use crate::media::track::track_remote::TrackRemote;
use crate::peer::configuration::Configuration;
use crate::peer::ice::ice_connection_state::ICEConnectionState;
use crate::peer::ice::ice_gather::ice_gatherer::{
    ICEGatherer, OnGatheringCompleteHdlrFn, OnICEGathererStateChangeHdlrFn, OnLocalCandidateHdlrFn,
};
use crate::peer::ice::ice_gather::ICEGatherOptions;
use crate::peer::peer_connection_state::{NegotiationNeededState, PeerConnectionState};
use crate::peer::policy::bundle_policy::BundlePolicy;
use crate::peer::policy::ice_transport_policy::ICETransportPolicy;
use crate::peer::policy::rtcp_mux_policy::RTCPMuxPolicy;
use crate::peer::policy::sdp_semantics::SDPSemantics;
use crate::peer::sdp::session_description::{SessionDescription, SessionDescriptionSerde};
use crate::peer::signaling_state::{check_next_signaling_state, SignalingState, StateChangeOp};

use crate::data::data_channel::data_channel_config::DataChannelConfig;
use crate::data::data_channel::data_channel_parameters::DataChannelParameters;
use crate::data::data_channel::data_channel_state::DataChannelState;
use crate::data::sctp_transport::sctp_transport_capabilities::SCTPTransportCapabilities;
use crate::data::sctp_transport::sctp_transport_state::SCTPTransportState;
use crate::error::Error;
use crate::media::dtls_transport::dtls_fingerprint::DTLSFingerprint;
use crate::media::dtls_transport::dtls_parameters::DTLSParameters;
use crate::media::dtls_transport::dtls_role::{
    DTLSRole, DEFAULT_DTLS_ROLE_ANSWER, DEFAULT_DTLS_ROLE_OFFER,
};
use crate::media::rtp::rtp_codec::{RTPCodecType, RTPHeaderExtensionCapability};
use crate::media::rtp::rtp_sender::RTPSender;
use crate::media::rtp::rtp_transceiver_direction::RTPTransceiverDirection;
use crate::media::rtp::{RTPTransceiverInit, SSRC};
use crate::media::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use crate::media::track::track_local::TrackLocal;
use crate::peer::ice::ice_candidate::{ICECandidate, ICECandidateInit};
use crate::peer::ice::ice_gather::ice_gatherer_state::ICEGathererState;
use crate::peer::ice::ice_gather::ice_gathering_state::ICEGatheringState;
use crate::peer::ice::ice_role::ICERole;
use crate::peer::ice::ICEParameters;
use crate::peer::offer_answer_options::{AnswerOptions, OfferOptions};
use crate::peer::operation::{Operation, Operations};
use crate::peer::sdp::sdp_type::SDPType;
use crate::peer::sdp::*;
use crate::util::{flatten_errs, math_rand_alpha};
use crate::{
    MEDIA_SECTION_APPLICATION, RECEIVE_MTU, SIMULCAST_MAX_PROBE_ROUTINES, SIMULCAST_PROBE_COUNT,
    SSRC_STR,
};
use anyhow::Result;
use ice::candidate::candidate_base::unmarshal_candidate;
use ice::candidate::Candidate;
use sdp::session_description::{ATTR_KEY_ICELITE, ATTR_KEY_MSID};
use sdp::util::ConnectionRole;
use srtp::stream::Stream;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Mutex};

use crate::media::dtls_transport::dtls_certificate::Certificate;
use peer_connection_internal::*;
use rcgen::KeyPair;

pub type OnSignalingStateChangeHdlrFn = Box<
    dyn (FnMut(SignalingState) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync,
>;

pub type OnICEConnectionStateChangeHdlrFn = Box<
    dyn (FnMut(ICEConnectionState) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnPeerConnectionStateChangeHdlrFn = Box<
    dyn (FnMut(PeerConnectionState) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnDataChannelHdlrFn = Box<
    dyn (FnMut(Arc<DataChannel>) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnTrackHdlrFn = Box<
    dyn (FnMut(
            Option<Arc<TrackRemote>>,
            Option<Arc<RTPReceiver>>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>)
        + Send
        + Sync,
>;

pub type OnNegotiationNeededHdlrFn =
    Box<dyn (FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>) + Send + Sync>;

#[derive(Clone)]
struct StartTransportsParams {
    ice_transport: Arc<ICETransport>,
    dtls_transport: Arc<DTLSTransport>,
    on_peer_connection_state_change_handler: Arc<Mutex<Option<OnPeerConnectionStateChangeHdlrFn>>>,
    is_closed: Arc<AtomicBool>,
    peer_connection_state: Arc<AtomicU8>,
    ice_connection_state: Arc<AtomicU8>,
}

#[derive(Clone)]
struct CheckNegotiationNeededParams {
    sctp_transport: Arc<SCTPTransport>,
    rtp_transceivers: Arc<Mutex<Vec<Arc<RTPTransceiver>>>>,
    current_local_description: Arc<Mutex<Option<SessionDescription>>>,
    current_remote_description: Arc<Mutex<Option<SessionDescription>>>,
}

#[derive(Clone)]
struct NegotiationNeededParams {
    on_negotiation_needed_handler: Arc<Mutex<Option<OnNegotiationNeededHdlrFn>>>,
    is_closed: Arc<AtomicBool>,
    ops: Arc<Operations>,
    negotiation_needed_state: Arc<AtomicU8>,
    is_negotiation_needed: Arc<AtomicBool>,
    signaling_state: Arc<AtomicU8>,
    check_negotiation_needed_params: CheckNegotiationNeededParams,
}

/// PeerConnection represents a WebRTC connection that establishes a
/// peer-to-peer communications with another PeerConnection instance in a
/// browser, or to another endpoint implementing the required protocols.
#[derive(Default)]
pub struct PeerConnection {
    stats_id: String,

    sdp_origin: sdp::session_description::Origin,

    configuration: Configuration,

    idp_login_url: Option<String>,

    last_offer: String,
    last_answer: String,

    /// a value containing the last known greater mid value
    /// we internally generate mids as numbers. Needed since JSEP
    /// requires that when reusing a media section a new unique mid
    /// should be defined (see JSEP 3.4.1).
    greater_mid: isize,

    interceptor_rtcp_writer: Option<Arc<dyn RTCPWriter + Send + Sync>>,

    pub(crate) internal: Arc<PeerConnectionInternal>,
}

impl PeerConnection {
    /// creates a PeerConnection with the default codecs and
    /// interceptors.  See register_default_codecs and RegisterDefaultInterceptors.
    ///
    /// If you wish to customize the set of available codecs or the set of
    /// active interceptors, create a MediaEngine and call api.new_peer_connection
    /// instead of this function.
    pub(crate) async fn new(api: &API, mut configuration: Configuration) -> Result<Self> {
        PeerConnection::init_configuration(&mut configuration)?;

        // https://w3c.github.io/webrtc-pc/#constructor (Step #2)
        // Some variables defined explicitly despite their implicit zero values to
        // allow better readability to understand what is happening.
        Ok(PeerConnection {
            stats_id: format!(
                "PeerConnection-{}",
                SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
            ),
            last_offer: "".to_owned(),
            last_answer: "".to_owned(),
            greater_mid: -1,
            internal: Arc::new(PeerConnectionInternal::new(api, &mut configuration).await?),
            configuration,
            ..Default::default()
        })
    }

    /// init_configuration defines validation of the specified Configuration and
    /// its assignment to the internal configuration variable. This function differs
    /// from its set_configuration counterpart because most of the checks do not
    /// include verification statements related to the existing state. Thus the
    /// function describes only minor verification of some the struct variables.
    fn init_configuration(configuration: &mut Configuration) -> Result<()> {
        let sanitized_ice_servers = configuration.get_ice_servers();
        if !sanitized_ice_servers.is_empty() {
            for server in &sanitized_ice_servers {
                server.validate()?;
            }
        }

        // https://www.w3.org/TR/webrtc/#constructor (step #3)
        if !configuration.certificates.is_empty() {
            let now = SystemTime::now();
            for cert in &configuration.certificates {
                if cert.expires().duration_since(now).is_err() {
                    return Err(Error::ErrCertificateExpired.into());
                }
            }
        } else {
            let kp = KeyPair::generate(&rcgen::PKCS_ECDSA_P256_SHA256)?;
            let cert = Certificate::from_key_pair(kp)?;
            configuration.certificates = vec![cert];
        };

        Ok(())
    }

    /// on_signaling_state_change sets an event handler which is invoked when the
    /// peer connection's signaling state changes
    pub async fn on_signaling_state_change(&self, f: OnSignalingStateChangeHdlrFn) {
        let mut on_signaling_state_change_handler =
            self.internal.on_signaling_state_change_handler.lock().await;
        *on_signaling_state_change_handler = Some(f);
    }

    async fn do_signaling_state_change(&self, new_state: SignalingState) {
        log::info!("signaling state changed to {}", new_state);
        let mut handler = self.internal.on_signaling_state_change_handler.lock().await;
        if let Some(f) = &mut *handler {
            f(new_state).await;
        }
    }

    /// on_data_channel sets an event handler which is invoked when a data
    /// channel message arrives from a remote peer.
    pub async fn on_data_channel(&self, f: OnDataChannelHdlrFn) {
        let mut on_data_channel_handler = self.internal.on_data_channel_handler.lock().await;
        *on_data_channel_handler = Some(f);
    }

    /// on_negotiation_needed sets an event handler which is invoked when
    /// a change has occurred which requires session negotiation
    pub async fn on_negotiation_needed(&self, f: OnNegotiationNeededHdlrFn) {
        let mut on_negotiation_needed_handler =
            self.internal.on_negotiation_needed_handler.lock().await;
        *on_negotiation_needed_handler = Some(f);
    }

    fn do_negotiation_needed_inner(params: &NegotiationNeededParams) -> bool {
        // https://w3c.github.io/webrtc-pc/#updating-the-negotiation-needed-flag
        // non-canon step 1
        let state: NegotiationNeededState = params
            .negotiation_needed_state
            .load(Ordering::SeqCst)
            .into();
        if state == NegotiationNeededState::Run {
            params
                .negotiation_needed_state
                .store(NegotiationNeededState::Queue as u8, Ordering::SeqCst);
            false
        } else if state == NegotiationNeededState::Queue {
            false
        } else {
            params
                .negotiation_needed_state
                .store(NegotiationNeededState::Run as u8, Ordering::SeqCst);
            true
        }
    }
    /// do_negotiation_needed enqueues negotiation_needed_op if necessary
    /// caller of this method should hold `pc.mu` lock
    async fn do_negotiation_needed(params: NegotiationNeededParams) {
        if !PeerConnection::do_negotiation_needed_inner(&params) {
            return;
        }

        let params2 = params.clone();
        let _ = params
            .ops
            .enqueue(Operation(Box::new(move || {
                let params3 = params2.clone();
                Box::pin(async move { PeerConnection::negotiation_needed_op(params3).await })
            })))
            .await;
    }

    async fn after_negotiation_needed_op(params: NegotiationNeededParams) -> bool {
        if params.negotiation_needed_state.load(Ordering::SeqCst)
            == NegotiationNeededState::Queue as u8
        {
            PeerConnection::do_negotiation_needed_inner(&params)
        } else {
            params
                .negotiation_needed_state
                .store(NegotiationNeededState::Empty as u8, Ordering::SeqCst);
            false
        }
    }

    async fn negotiation_needed_op(params: NegotiationNeededParams) -> bool {
        // Don't run NegotiatedNeeded checks if on_negotiation_needed is not set
        {
            let handler = params.on_negotiation_needed_handler.lock().await;
            if handler.is_none() {
                return false;
            }
        }

        // https://www.w3.org/TR/webrtc/#updating-the-negotiation-needed-flag
        // Step 2.1
        if params.is_closed.load(Ordering::SeqCst) {
            return false;
        }
        // non-canon step 2.2
        if !params.ops.is_empty().await {
            //enqueue negotiation_needed_op again by return true
            return true;
        }

        // non-canon, run again if there was a request
        // starting defer(after_do_negotiation_needed(params).await);

        // Step 2.3
        if params.signaling_state.load(Ordering::SeqCst) != SignalingState::Stable as u8 {
            return PeerConnection::after_negotiation_needed_op(params).await;
        }

        // Step 2.4
        if !PeerConnection::check_negotiation_needed(&params.check_negotiation_needed_params).await
        {
            params.is_negotiation_needed.store(false, Ordering::SeqCst);
            return PeerConnection::after_negotiation_needed_op(params).await;
        }

        // Step 2.5
        if params.is_negotiation_needed.load(Ordering::SeqCst) {
            return PeerConnection::after_negotiation_needed_op(params).await;
        }

        // Step 2.6
        params.is_negotiation_needed.store(true, Ordering::SeqCst);

        // Step 2.7
        {
            let mut handler = params.on_negotiation_needed_handler.lock().await;
            if let Some(f) = &mut *handler {
                f().await;
            }
        }

        PeerConnection::after_negotiation_needed_op(params).await
    }

    async fn check_negotiation_needed(params: &CheckNegotiationNeededParams) -> bool {
        // To check if negotiation is needed for connection, perform the following checks:
        // Skip 1, 2 steps
        // Step 3
        let current_local_description = {
            let current_local_description = params.current_local_description.lock().await;
            current_local_description.clone()
        };

        if let Some(local_desc) = &current_local_description {
            let len_data_channel = {
                let data_channels = params.sctp_transport.data_channels.lock().await;
                data_channels.len()
            };

            if len_data_channel != 0 && have_data_channel(local_desc).is_none() {
                return true;
            }

            let transceivers = params.rtp_transceivers.lock().await;
            for t in &*transceivers {
                // https://www.w3.org/TR/webrtc/#dfn-update-the-negotiation-needed-flag
                // Step 5.1
                // if t.stopping && !t.stopped {
                // 	return true
                // }
                let m = get_by_mid(t.mid().await.as_str(), local_desc);
                // Step 5.2
                if !t.stopped && m.is_none() {
                    return true;
                }
                if !t.stopped {
                    if let Some(m) = m {
                        // Step 5.3.1
                        if t.direction() == RTPTransceiverDirection::Sendrecv
                            || t.direction() == RTPTransceiverDirection::Sendonly
                        {
                            if let (Some(desc_msid), Some(sender)) =
                                (m.attribute(ATTR_KEY_MSID), t.sender().await)
                            {
                                if let Some(track) = &sender.track().await {
                                    if desc_msid.as_str()
                                        != track.stream_id().to_owned() + " " + track.id()
                                    {
                                        return true;
                                    }
                                }
                            } else {
                                return true;
                            }
                        }
                        match local_desc.serde.sdp_type {
                            SDPType::Offer => {
                                // Step 5.3.2
                                let current_remote_description =
                                    params.current_remote_description.lock().await;
                                if let Some(remote_desc) = &*current_remote_description {
                                    if let Some(rm) =
                                        get_by_mid(t.mid().await.as_str(), remote_desc)
                                    {
                                        if get_peer_direction(m) != t.direction()
                                            && get_peer_direction(rm) != t.direction().reverse()
                                        {
                                            return true;
                                        }
                                    } else {
                                        return true;
                                    }
                                }
                            }
                            SDPType::Answer => {
                                // Step 5.3.3
                                if m.attribute(t.direction().to_string().as_str()).is_none() {
                                    return true;
                                }
                            }
                            _ => {}
                        };
                    }
                }
                // Step 5.4
                if t.stopped && !t.mid().await.is_empty() {
                    let current_remote_description = params.current_remote_description.lock().await;
                    if let Some(remote_desc) = &*current_remote_description {
                        if get_by_mid(t.mid().await.as_str(), local_desc).is_some()
                            || get_by_mid(t.mid().await.as_str(), remote_desc).is_some()
                        {
                            return true;
                        }
                    }
                }
            }
            // Step 6
            false
        } else {
            true
        }
    }

    /// on_ice_candidate sets an event handler which is invoked when a new ICE
    /// candidate is found.
    /// Take note that the handler is gonna be called with a nil pointer when
    /// gathering is finished.
    pub async fn on_ice_candidate(&self, f: OnLocalCandidateHdlrFn) {
        self.internal.ice_gatherer.on_local_candidate(f).await
    }

    /// on_ice_gathering_state_change sets an event handler which is invoked when the
    /// ICE candidate gathering state has changed.
    pub async fn on_ice_gathering_state_change(&self, f: OnICEGathererStateChangeHdlrFn) {
        self.internal.ice_gatherer.on_state_change(f).await
    }

    /// on_track sets an event handler which is called when remote track
    /// arrives from a remote peer.
    pub async fn on_track(&self, f: OnTrackHdlrFn) {
        let mut on_track_handler = self.internal.on_track_handler.lock().await;
        *on_track_handler = Some(f);
    }

    async fn do_track(
        on_track_handler: Arc<Mutex<Option<OnTrackHdlrFn>>>,
        t: Option<Arc<TrackRemote>>,
        r: Option<Arc<RTPReceiver>>,
    ) {
        log::debug!(
            "got new track: {}",
            if let Some(t) = &t {
                t.id().await
            } else {
                "None".to_owned()
            }
        );

        if t.is_some() {
            let mut handler = on_track_handler.lock().await;
            if let Some(f) = &mut *handler {
                f(t, r).await;
            } else {
                log::warn!("on_track unset, unable to handle incoming media streams");
            }
        }
    }

    /// on_ice_connection_state_change sets an event handler which is called
    /// when an ICE connection state is changed.
    pub async fn on_ice_connection_state_change(&self, f: OnICEConnectionStateChangeHdlrFn) {
        let mut on_ice_connection_state_change_handler = self
            .internal
            .on_ice_connection_state_change_handler
            .lock()
            .await;
        *on_ice_connection_state_change_handler = Some(f);
    }

    async fn do_ice_connection_state_change(
        on_ice_connection_state_change_handler: &Arc<
            Mutex<Option<OnICEConnectionStateChangeHdlrFn>>,
        >,
        ice_connection_state: &Arc<AtomicU8>,
        cs: ICEConnectionState,
    ) {
        ice_connection_state.store(cs as u8, Ordering::SeqCst);

        log::info!("ICE connection state changed: {}", cs);
        let mut handler = on_ice_connection_state_change_handler.lock().await;
        if let Some(f) = &mut *handler {
            f(cs).await;
        }
    }

    /// on_peer_connection_state_change sets an event handler which is called
    /// when the PeerConnectionState has changed
    pub async fn on_peer_connection_state_change(&self, f: OnPeerConnectionStateChangeHdlrFn) {
        let mut on_peer_connection_state_change_handler = self
            .internal
            .on_peer_connection_state_change_handler
            .lock()
            .await;
        *on_peer_connection_state_change_handler = Some(f);
    }

    async fn do_peer_connection_state_change(
        on_peer_connection_state_change_handler: &Arc<
            Mutex<Option<OnPeerConnectionStateChangeHdlrFn>>,
        >,
        cs: PeerConnectionState,
    ) {
        let mut handler = on_peer_connection_state_change_handler.lock().await;
        if let Some(f) = &mut *handler {
            f(cs).await;
        }
    }

    /// set_configuration updates the configuration of this PeerConnection object.
    pub async fn set_configuration(&mut self, configuration: Configuration) -> Result<()> {
        //nolint:gocognit
        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-setconfiguration (step #2)
        if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        }

        // https://www.w3.org/TR/webrtc/#set-the-configuration (step #3)
        if !configuration.peer_identity.is_empty() {
            if configuration.peer_identity != self.configuration.peer_identity {
                return Err(Error::ErrModifyingPeerIdentity.into());
            }
            self.configuration.peer_identity = configuration.peer_identity;
        }

        // https://www.w3.org/TR/webrtc/#set-the-configuration (step #4)
        if !configuration.certificates.is_empty() {
            if configuration.certificates.len() != self.configuration.certificates.len() {
                return Err(Error::ErrModifyingCertificates.into());
            }

            self.configuration.certificates = configuration.certificates;
        }

        // https://www.w3.org/TR/webrtc/#set-the-configuration (step #5)
        if configuration.bundle_policy != BundlePolicy::Unspecified {
            if configuration.bundle_policy != self.configuration.bundle_policy {
                return Err(Error::ErrModifyingBundlePolicy.into());
            }
            self.configuration.bundle_policy = configuration.bundle_policy;
        }

        // https://www.w3.org/TR/webrtc/#set-the-configuration (step #6)
        if configuration.rtcp_mux_policy != RTCPMuxPolicy::Unspecified {
            if configuration.rtcp_mux_policy != self.configuration.rtcp_mux_policy {
                return Err(Error::ErrModifyingRTCPMuxPolicy.into());
            }
            self.configuration.rtcp_mux_policy = configuration.rtcp_mux_policy;
        }

        // https://www.w3.org/TR/webrtc/#set-the-configuration (step #7)
        if configuration.ice_candidate_pool_size != 0 {
            if self.configuration.ice_candidate_pool_size != configuration.ice_candidate_pool_size
                && self.local_description().await.is_some()
            {
                return Err(Error::ErrModifyingICECandidatePoolSize.into());
            }
            self.configuration.ice_candidate_pool_size = configuration.ice_candidate_pool_size;
        }

        // https://www.w3.org/TR/webrtc/#set-the-configuration (step #8)
        if configuration.ice_transport_policy != ICETransportPolicy::Unspecified {
            self.configuration.ice_transport_policy = configuration.ice_transport_policy
        }

        // https://www.w3.org/TR/webrtc/#set-the-configuration (step #11)
        if !configuration.ice_servers.is_empty() {
            // https://www.w3.org/TR/webrtc/#set-the-configuration (step #11.3)
            for server in &configuration.ice_servers {
                server.validate()?;
            }
            self.configuration.ice_servers = configuration.ice_servers
        }
        Ok(())
    }

    /// get_configuration returns a Configuration object representing the current
    /// configuration of this PeerConnection object. The returned object is a
    /// copy and direct mutation on it will not take affect until set_configuration
    /// has been called with Configuration passed as its only argument.
    /// https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-getconfiguration
    pub fn get_configuration(&self) -> &Configuration {
        &self.configuration
    }

    fn get_stats_id(&self) -> &str {
        self.stats_id.as_str()
    }

    /// create_offer starts the PeerConnection and generates the localDescription
    /// https://w3c.github.io/webrtc-pc/#dom-rtcpeerconnection-createoffer
    pub async fn create_offer(
        &mut self,
        options: Option<OfferOptions>,
    ) -> Result<SessionDescription> {
        let use_identity = self.idp_login_url.is_some();
        if use_identity {
            return Err(Error::ErrIdentityProviderNotImplemented.into());
        } else if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        }

        if let Some(options) = options {
            if options.ice_restart {
                self.internal.ice_transport.restart().await?;
            }
        }

        // This may be necessary to recompute if, for example, createOffer was called when only an
        // audio RTCRtpTransceiver was added to connection, but while performing the in-parallel
        // steps to create an offer, a video RTCRtpTransceiver was added, requiring additional
        // inspection of video system resources.
        let mut count = 0;
        let mut offer;

        loop {
            // We cache current transceivers to ensure they aren't
            // mutated during offer generation. We later check if they have
            // been mutated and recompute the offer if necessary.
            let current_transceivers = {
                let rtp_transceivers = self.internal.rtp_transceivers.lock().await;
                rtp_transceivers.clone()
            };

            // in-parallel steps to create an offer
            // https://w3c.github.io/webrtc-pc/#dfn-in-parallel-steps-to-create-an-offer
            let is_plan_b = {
                let current_remote_description =
                    self.internal.current_remote_description.lock().await;
                if current_remote_description.is_some() {
                    description_is_plan_b(current_remote_description.as_ref())?
                } else {
                    self.configuration.sdp_semantics == SDPSemantics::PlanB
                }
            };

            // include unmatched local transceivers
            if !is_plan_b {
                // update the greater mid if the remote description provides a greater one
                {
                    let current_remote_description =
                        self.internal.current_remote_description.lock().await;
                    if let Some(d) = &*current_remote_description {
                        if let Some(parsed) = &d.parsed {
                            for media in &parsed.media_descriptions {
                                if let Some(mid) = get_mid_value(media) {
                                    if mid.is_empty() {
                                        continue;
                                    }
                                    let numeric_mid = match mid.parse::<isize>() {
                                        Ok(n) => n,
                                        Err(_) => continue,
                                    };
                                    if numeric_mid > self.greater_mid {
                                        self.greater_mid = numeric_mid;
                                    }
                                }
                            }
                        }
                    }
                }
                for t in &current_transceivers {
                    if !t.mid().await.is_empty() {
                        continue;
                    }
                    self.greater_mid += 1;
                    t.set_mid(format!("{}", self.greater_mid)).await?;
                }
            }

            let current_remote_description_is_none = {
                let current_remote_description =
                    self.internal.current_remote_description.lock().await;
                current_remote_description.is_none()
            };

            let mut d = if current_remote_description_is_none {
                self.internal
                    .generate_unmatched_sdp(
                        current_transceivers,
                        use_identity,
                        self.configuration.sdp_semantics,
                    )
                    .await?
            } else {
                self.internal
                    .generate_matched_sdp(
                        current_transceivers,
                        use_identity,
                        true, /*includeUnmatched */
                        DEFAULT_DTLS_ROLE_OFFER.to_connection_role(),
                        self.configuration.sdp_semantics,
                    )
                    .await?
            };

            update_sdp_origin(&mut self.sdp_origin, &mut d);
            let sdp = d.marshal();

            offer = SessionDescription {
                serde: SessionDescriptionSerde {
                    sdp_type: SDPType::Offer,
                    sdp,
                },
                parsed: Some(d),
            };

            // Verify local media hasn't changed during offer
            // generation. Recompute if necessary
            if is_plan_b || !self.internal.has_local_description_changed(&offer).await {
                break;
            }
            count += 1;
            if count >= 128 {
                return Err(Error::ErrExcessiveRetries.into());
            }
        }

        self.last_offer = offer.serde.sdp.clone();
        Ok(offer)
    }

    /// Update the PeerConnectionState given the state of relevant transports
    /// https://www.w3.org/TR/webrtc/#rtcpeerconnectionstate-enum
    async fn update_connection_state(
        on_peer_connection_state_change_handler: &Arc<
            Mutex<Option<OnPeerConnectionStateChangeHdlrFn>>,
        >,
        is_closed: &Arc<AtomicBool>,
        peer_connection_state: &Arc<AtomicU8>,
        ice_connection_state: ICEConnectionState,
        dtls_transport_state: DTLSTransportState,
    ) {
        let  connection_state =
        // The RTCPeerConnection object's [[IsClosed]] slot is true.
        if is_closed.load(Ordering::SeqCst) {
             PeerConnectionState::Closed
        }else if ice_connection_state == ICEConnectionState::Failed || dtls_transport_state == DTLSTransportState::Failed {
            // Any of the RTCIceTransports or RTCDtlsTransports are in a "failed" state.
             PeerConnectionState::Failed
        }else if ice_connection_state == ICEConnectionState::Disconnected {
            // Any of the RTCIceTransports or RTCDtlsTransports are in the "disconnected"
            // state and none of them are in the "failed" or "connecting" or "checking" state.
            PeerConnectionState::Disconnected
        }else if ice_connection_state == ICEConnectionState::Connected && dtls_transport_state == DTLSTransportState::Connected {
            // All RTCIceTransports and RTCDtlsTransports are in the "connected", "completed" or "closed"
            // state and at least one of them is in the "connected" or "completed" state.
            PeerConnectionState::Connected
        }else if ice_connection_state == ICEConnectionState::Checking && dtls_transport_state == DTLSTransportState::Connecting{
        //  Any of the RTCIceTransports or RTCDtlsTransports are in the "connecting" or
        // "checking" state and none of them is in the "failed" state.
             PeerConnectionState::Connecting
        }else{
            PeerConnectionState::New
        };

        if peer_connection_state.load(Ordering::SeqCst) == connection_state as u8 {
            return;
        }

        log::info!("peer connection state changed: {}", connection_state);
        peer_connection_state.store(connection_state as u8, Ordering::SeqCst);

        PeerConnection::do_peer_connection_state_change(
            on_peer_connection_state_change_handler,
            connection_state,
        )
        .await;
    }

    /// create_answer starts the PeerConnection and generates the localDescription
    pub async fn create_answer(
        &mut self,
        _options: Option<AnswerOptions>,
    ) -> Result<SessionDescription> {
        let use_identity = self.idp_login_url.is_some();
        if self.remote_description().await.is_none() {
            return Err(Error::ErrNoRemoteDescription.into());
        } else if use_identity {
            return Err(Error::ErrIdentityProviderNotImplemented.into());
        } else if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        } else if self.signaling_state() != SignalingState::HaveRemoteOffer
            && self.signaling_state() != SignalingState::HaveLocalPranswer
        {
            return Err(Error::ErrIncorrectSignalingState.into());
        }

        let mut connection_role = self
            .internal
            .setting_engine
            .answering_dtls_role
            .to_connection_role();
        if connection_role == ConnectionRole::Unspecified {
            connection_role = DEFAULT_DTLS_ROLE_ANSWER.to_connection_role();
        }

        let local_transceivers = self.get_transceivers().await;
        let mut d = self
            .internal
            .generate_matched_sdp(
                local_transceivers,
                use_identity,
                false, /*includeUnmatched */
                connection_role,
                self.configuration.sdp_semantics,
            )
            .await?;

        update_sdp_origin(&mut self.sdp_origin, &mut d);
        let sdp = d.marshal();

        let answer = SessionDescription {
            serde: SessionDescriptionSerde {
                sdp_type: SDPType::Answer,
                sdp,
            },
            parsed: Some(d),
        };

        self.last_answer = answer.serde.sdp.clone();
        Ok(answer)
    }

    // 4.4.1.6 Set the SessionDescription
    pub(crate) async fn set_description(
        &mut self,
        sd: &SessionDescription,
        op: StateChangeOp,
    ) -> Result<()> {
        if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        } else if sd.serde.sdp_type == SDPType::Unspecified {
            return Err(Error::ErrPeerConnSDPTypeInvalidValue.into());
        }

        let next_state = {
            let cur = self.signaling_state();
            let new_sdpdoes_not_match_offer = Error::ErrSDPDoesNotMatchOffer;
            let new_sdpdoes_not_match_answer = Error::ErrSDPDoesNotMatchAnswer;

            match op {
                StateChangeOp::SetLocal => {
                    match sd.serde.sdp_type {
                        // stable->SetLocal(offer)->have-local-offer
                        SDPType::Offer => {
                            if sd.serde.sdp != self.last_offer {
                                Err(new_sdpdoes_not_match_offer.into())
                            } else {
                                let next_state = check_next_signaling_state(
                                    cur,
                                    SignalingState::HaveLocalOffer,
                                    StateChangeOp::SetLocal,
                                    sd.serde.sdp_type,
                                );
                                if next_state.is_ok() {
                                    let mut pending_local_description =
                                        self.internal.pending_local_description.lock().await;
                                    *pending_local_description = Some(sd.clone());
                                }
                                next_state
                            }
                        }
                        // have-remote-offer->SetLocal(answer)->stable
                        // have-local-pranswer->SetLocal(answer)->stable
                        SDPType::Answer => {
                            if sd.serde.sdp != self.last_answer {
                                Err(new_sdpdoes_not_match_answer.into())
                            } else {
                                let next_state = check_next_signaling_state(
                                    cur,
                                    SignalingState::Stable,
                                    StateChangeOp::SetLocal,
                                    sd.serde.sdp_type,
                                );
                                if next_state.is_ok() {
                                    let pending_remote_description = {
                                        let mut pending_remote_description =
                                            self.internal.pending_remote_description.lock().await;
                                        pending_remote_description.take()
                                    };
                                    let _pending_local_description = {
                                        let mut pending_local_description =
                                            self.internal.pending_local_description.lock().await;
                                        pending_local_description.take()
                                    };

                                    {
                                        let mut current_local_description =
                                            self.internal.current_local_description.lock().await;
                                        *current_local_description = Some(sd.clone());
                                    }
                                    {
                                        let mut current_remote_description =
                                            self.internal.current_remote_description.lock().await;
                                        *current_remote_description = pending_remote_description;
                                    }
                                }
                                next_state
                            }
                        }
                        SDPType::Rollback => {
                            let next_state = check_next_signaling_state(
                                cur,
                                SignalingState::Stable,
                                StateChangeOp::SetLocal,
                                sd.serde.sdp_type,
                            );
                            if next_state.is_ok() {
                                let mut pending_local_description =
                                    self.internal.pending_local_description.lock().await;
                                *pending_local_description = None;
                            }
                            next_state
                        }
                        // have-remote-offer->SetLocal(pranswer)->have-local-pranswer
                        SDPType::Pranswer => {
                            if sd.serde.sdp != self.last_answer {
                                Err(new_sdpdoes_not_match_answer.into())
                            } else {
                                let next_state = check_next_signaling_state(
                                    cur,
                                    SignalingState::HaveLocalPranswer,
                                    StateChangeOp::SetLocal,
                                    sd.serde.sdp_type,
                                );
                                if next_state.is_ok() {
                                    let mut pending_local_description =
                                        self.internal.pending_local_description.lock().await;
                                    *pending_local_description = Some(sd.clone());
                                }
                                next_state
                            }
                        }
                        _ => Err(Error::ErrPeerConnStateChangeInvalid.into()),
                    }
                }
                StateChangeOp::SetRemote => {
                    match sd.serde.sdp_type {
                        // stable->SetRemote(offer)->have-remote-offer
                        SDPType::Offer => {
                            let next_state = check_next_signaling_state(
                                cur,
                                SignalingState::HaveRemoteOffer,
                                StateChangeOp::SetRemote,
                                sd.serde.sdp_type,
                            );
                            if next_state.is_ok() {
                                let mut pending_remote_description =
                                    self.internal.pending_remote_description.lock().await;
                                *pending_remote_description = Some(sd.clone());
                            }
                            next_state
                        }
                        // have-local-offer->SetRemote(answer)->stable
                        // have-remote-pranswer->SetRemote(answer)->stable
                        SDPType::Answer => {
                            let next_state = check_next_signaling_state(
                                cur,
                                SignalingState::Stable,
                                StateChangeOp::SetRemote,
                                sd.serde.sdp_type,
                            );
                            if next_state.is_ok() {
                                let pending_local_description = {
                                    let mut pending_local_description =
                                        self.internal.pending_local_description.lock().await;
                                    pending_local_description.take()
                                };

                                let _pending_remote_description = {
                                    let mut pending_remote_description =
                                        self.internal.pending_remote_description.lock().await;
                                    pending_remote_description.take()
                                };

                                {
                                    let mut current_remote_description =
                                        self.internal.current_remote_description.lock().await;
                                    *current_remote_description = Some(sd.clone());
                                }
                                {
                                    let mut current_local_description =
                                        self.internal.current_local_description.lock().await;
                                    *current_local_description = pending_local_description;
                                }
                            }
                            next_state
                        }
                        SDPType::Rollback => {
                            let next_state = check_next_signaling_state(
                                cur,
                                SignalingState::Stable,
                                StateChangeOp::SetRemote,
                                sd.serde.sdp_type,
                            );
                            if next_state.is_ok() {
                                let mut pending_remote_description =
                                    self.internal.pending_remote_description.lock().await;
                                *pending_remote_description = None;
                            }
                            next_state
                        }
                        // have-local-offer->SetRemote(pranswer)->have-remote-pranswer
                        SDPType::Pranswer => {
                            let next_state = check_next_signaling_state(
                                cur,
                                SignalingState::HaveRemotePranswer,
                                StateChangeOp::SetRemote,
                                sd.serde.sdp_type,
                            );
                            if next_state.is_ok() {
                                let mut pending_remote_description =
                                    self.internal.pending_remote_description.lock().await;
                                *pending_remote_description = Some(sd.clone());
                            }
                            next_state
                        }
                        _ => Err(Error::ErrPeerConnStateChangeInvalid.into()),
                    }
                } //_ => Err(Error::ErrPeerConnStateChangeUnhandled.into()),
            }
        };

        match next_state {
            Ok(next_state) => {
                self.internal
                    .signaling_state
                    .store(next_state as u8, Ordering::SeqCst);
                if self.signaling_state() == SignalingState::Stable {
                    self.internal
                        .is_negotiation_needed
                        .store(false, Ordering::SeqCst);
                    PeerConnection::do_negotiation_needed(NegotiationNeededParams {
                        on_negotiation_needed_handler: Arc::clone(
                            &self.internal.on_negotiation_needed_handler,
                        ),
                        is_closed: Arc::clone(&self.internal.is_closed),
                        ops: Arc::clone(&self.internal.ops),
                        negotiation_needed_state: Arc::clone(
                            &self.internal.negotiation_needed_state,
                        ),
                        is_negotiation_needed: Arc::clone(&self.internal.is_negotiation_needed),
                        signaling_state: Arc::clone(&self.internal.signaling_state),
                        check_negotiation_needed_params: CheckNegotiationNeededParams {
                            sctp_transport: Arc::clone(&self.internal.sctp_transport),
                            rtp_transceivers: Arc::clone(&self.internal.rtp_transceivers),
                            current_local_description: self
                                .internal
                                .current_local_description
                                .clone(),
                            current_remote_description: self
                                .internal
                                .current_remote_description
                                .clone(),
                        },
                    })
                    .await;
                }
                self.do_signaling_state_change(next_state).await;
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    /// set_local_description sets the SessionDescription of the local peer
    pub async fn set_local_description(&mut self, mut desc: SessionDescription) -> Result<()> {
        if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        }

        let have_local_description = {
            let current_local_description = self.internal.current_local_description.lock().await;
            current_local_description.is_some()
        };

        // JSEP 5.4
        if desc.serde.sdp.is_empty() {
            match desc.serde.sdp_type {
                SDPType::Answer | SDPType::Pranswer => {
                    desc.serde.sdp = self.last_answer.clone();
                }
                SDPType::Offer => {
                    desc.serde.sdp = self.last_offer.clone();
                }
                _ => return Err(Error::ErrPeerConnSDPTypeInvalidValueSetLocalDescription.into()),
            }
        }

        desc.parsed = Some(desc.unmarshal()?);
        self.set_description(&desc, StateChangeOp::SetLocal).await?;

        let we_answer = desc.serde.sdp_type == SDPType::Answer;
        let remote_description = self.remote_description().await;
        if we_answer {
            if let Some(remote_desc) = remote_description {
                self.start_rtp_senders().await?;

                let pci = Arc::clone(&self.internal);
                let sdp_semantics = self.configuration.sdp_semantics;
                let remote_desc = Arc::new(remote_desc);
                self.internal
                    .ops
                    .enqueue(Operation(Box::new(move || {
                        let pc = Arc::clone(&pci);
                        let rd = Arc::clone(&remote_desc);
                        Box::pin(async move {
                            let _ = pc
                                .start_rtp(have_local_description, rd, sdp_semantics)
                                .await;
                            false
                        })
                    })))
                    .await?;
            }
        }

        if self.internal.ice_gatherer.state() == ICEGathererState::New {
            self.internal.ice_gatherer.gather().await
        } else {
            Ok(())
        }
    }

    /// local_description returns PendingLocalDescription if it is not null and
    /// otherwise it returns CurrentLocalDescription. This property is used to
    /// determine if set_local_description has already been called.
    /// https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-localdescription
    pub async fn local_description(&self) -> Option<SessionDescription> {
        if let Some(pending_local_description) = self.pending_local_description().await {
            return Some(pending_local_description);
        }
        self.current_local_description().await
    }

    /// set_remote_description sets the SessionDescription of the remote peer
    pub async fn set_remote_description(&mut self, mut desc: SessionDescription) -> Result<()> {
        if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        }

        let is_renegotation = {
            let current_remote_description = self.internal.current_remote_description.lock().await;
            current_remote_description.is_some()
        };

        desc.parsed = Some(desc.unmarshal()?);
        self.set_description(&desc, StateChangeOp::SetRemote)
            .await?;

        if let Some(parsed) = &desc.parsed {
            self.internal
                .media_engine
                .update_from_remote_description(parsed)
                .await?;

            let mut local_transceivers = self.get_transceivers().await;
            let remote_description = self.remote_description().await;
            let detected_plan_b = description_is_plan_b(remote_description.as_ref())?;
            let we_offer = desc.serde.sdp_type == SDPType::Answer;

            if !we_offer && !detected_plan_b {
                if let Some(remote_desc) = remote_description {
                    if let Some(parsed) = &remote_desc.parsed {
                        for media in &parsed.media_descriptions {
                            if let Some(mid_value) = get_mid_value(media) {
                                if mid_value.is_empty() {
                                    return Err(
                                        Error::ErrPeerConnRemoteDescriptionWithoutMidValue.into()
                                    );
                                }

                                if media.media_name.media == MEDIA_SECTION_APPLICATION {
                                    continue;
                                }

                                let kind = RTPCodecType::from(media.media_name.media.as_str());
                                let direction = get_peer_direction(media);
                                if kind == RTPCodecType::Unspecified
                                    || direction == RTPTransceiverDirection::Unspecified
                                {
                                    continue;
                                }

                                let t = if let Some(t) =
                                    find_by_mid(mid_value, &mut local_transceivers).await
                                {
                                    t.stop().await?;
                                    Some(t)
                                } else {
                                    satisfy_type_and_direction(
                                        kind,
                                        direction,
                                        &mut local_transceivers,
                                    )
                                    .await
                                };

                                if let Some(t) = t {
                                    if direction == RTPTransceiverDirection::Recvonly {
                                        if t.direction() == RTPTransceiverDirection::Sendrecv {
                                            t.set_direction(RTPTransceiverDirection::Sendonly);
                                        }
                                    } else if direction == RTPTransceiverDirection::Sendrecv
                                        && t.direction() == RTPTransceiverDirection::Sendonly
                                    {
                                        t.set_direction(RTPTransceiverDirection::Sendrecv);
                                    }

                                    if t.mid().await.is_empty() {
                                        t.set_mid(mid_value.to_owned()).await?;
                                    }
                                } else {
                                    let receiver = Arc::new(RTPReceiver::new(
                                        kind,
                                        Arc::clone(&self.internal.dtls_transport),
                                        Arc::clone(&self.internal.media_engine),
                                        self.internal.interceptor.clone(),
                                    ));

                                    let local_direction =
                                        if direction == RTPTransceiverDirection::Recvonly {
                                            RTPTransceiverDirection::Sendonly
                                        } else {
                                            RTPTransceiverDirection::Recvonly
                                        };

                                    let t = Arc::new(RTPTransceiver::new(
                                        Some(receiver),
                                        None,
                                        local_direction,
                                        kind,
                                        vec![],
                                        Arc::clone(&self.internal.media_engine),
                                    ));

                                    self.internal.add_rtp_transceiver(Arc::clone(&t)).await;

                                    if t.mid().await.is_empty() {
                                        t.set_mid(mid_value.to_owned()).await?;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let (remote_ufrag, remote_pwd, candidates) = extract_ice_details(parsed).await?;

            if is_renegotation
                && self
                    .internal
                    .ice_transport
                    .have_remote_credentials_change(&remote_ufrag, &remote_pwd)
                    .await
            {
                // An ICE Restart only happens implicitly for a set_remote_description of type offer
                if !we_offer {
                    self.internal.ice_transport.restart().await?;
                }

                self.internal
                    .ice_transport
                    .set_remote_credentials(remote_ufrag.clone(), remote_pwd.clone())
                    .await?;
            }

            for candidate in candidates {
                self.internal
                    .ice_transport
                    .add_remote_candidate(Some(candidate))
                    .await?;
            }

            if is_renegotation {
                if we_offer {
                    self.start_rtp_senders().await?;

                    let pci = Arc::clone(&self.internal);
                    let sdp_semantics = self.configuration.sdp_semantics;
                    let remote_desc = Arc::new(desc);
                    self.internal
                        .ops
                        .enqueue(Operation(Box::new(move || {
                            let pc = Arc::clone(&pci);
                            let rd = Arc::clone(&remote_desc);
                            Box::pin(async move {
                                let _ = pc.start_rtp(true, rd, sdp_semantics).await;
                                false
                            })
                        })))
                        .await?;
                }
                return Ok(());
            }

            let mut remote_is_lite = false;
            for a in &parsed.attributes {
                if a.key.trim() == ATTR_KEY_ICELITE {
                    remote_is_lite = true;
                    break;
                }
            }

            let (fingerprint, fingerprint_hash) = extract_fingerprint(parsed)?;

            // If one of the agents is lite and the other one is not, the lite agent must be the controlling agent.
            // If both or neither agents are lite the offering agent is controlling.
            // RFC 8445 S6.1.1
            let ice_role = if (we_offer
                && remote_is_lite == self.internal.setting_engine.candidates.ice_lite)
                || (remote_is_lite && !self.internal.setting_engine.candidates.ice_lite)
            {
                ICERole::Controlling
            } else {
                ICERole::Controlled
            };

            // Start the networking in a new routine since it will block until
            // the connection is actually established.
            if we_offer {
                self.start_rtp_senders().await?;
            }

            //log::trace!("start_transports: parsed={:?}", parsed);

            let pci = Arc::clone(&self.internal);
            let sdp_semantics = self.configuration.sdp_semantics;
            let dtls_role = DTLSRole::from(parsed);
            let remote_desc = Arc::new(desc);
            self.internal
                .ops
                .enqueue(Operation(Box::new(move || {
                    let pc = Arc::clone(&pci);
                    let rd = Arc::clone(&remote_desc);
                    let ru = remote_ufrag.clone();
                    let rp = remote_pwd.clone();
                    let fp = fingerprint.clone();
                    let fp_hash = fingerprint_hash.clone();
                    Box::pin(async move {
                        log::trace!(
                            "start_transports: ice_role={}, dtls_role={}",
                            ice_role,
                            dtls_role,
                        );
                        pc.start_transports(ice_role, dtls_role, ru, rp, fp, fp_hash)
                            .await;

                        if we_offer {
                            let _ = pc.start_rtp(false, rd, sdp_semantics).await;
                        }
                        false
                    })
                })))
                .await?;
        }

        Ok(())
    }

    /// start_rtp_senders starts all outbound RTP streams
    pub(crate) async fn start_rtp_senders(&self) -> Result<()> {
        let current_transceivers = self.internal.rtp_transceivers.lock().await;
        for transceiver in &*current_transceivers {
            if let Some(sender) = transceiver.sender().await {
                if sender.is_negotiated() && !sender.has_sent().await {
                    sender.send(&sender.get_parameters().await).await?;
                }
            }
        }

        Ok(())
    }

    /// remote_description returns pending_remote_description if it is not null and
    /// otherwise it returns current_remote_description. This property is used to
    /// determine if setRemoteDescription has already been called.
    /// https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-remotedescription
    pub async fn remote_description(&self) -> Option<SessionDescription> {
        self.internal.remote_description().await
    }

    /// add_ice_candidate accepts an ICE candidate string and adds it
    /// to the existing set of candidates.
    pub async fn add_ice_candidate(&self, candidate: ICECandidateInit) -> Result<()> {
        if self.remote_description().await.is_none() {
            return Err(Error::ErrNoRemoteDescription.into());
        }

        let candidate_value = match candidate.candidate.strip_prefix("candidate:") {
            Some(s) => s,
            None => candidate.candidate.as_str(),
        };

        let ice_candidate = if !candidate_value.is_empty() {
            let candidate: Arc<dyn Candidate + Send + Sync> =
                Arc::new(unmarshal_candidate(candidate_value).await?);

            Some(ICECandidate::from(&candidate))
        } else {
            None
        };

        self.internal
            .ice_transport
            .add_remote_candidate(ice_candidate)
            .await
    }

    /// ice_connection_state returns the ICE connection state of the
    /// PeerConnection instance.
    pub fn ice_connection_state(&self) -> ICEConnectionState {
        self.internal
            .ice_connection_state
            .load(Ordering::SeqCst)
            .into()
    }

    /// get_senders returns the RTPSender that are currently attached to this PeerConnection
    pub async fn get_senders(&self) -> Vec<Arc<RTPSender>> {
        let mut senders = vec![];
        let rtp_transceivers = self.internal.rtp_transceivers.lock().await;
        for transceiver in &*rtp_transceivers {
            if let Some(sender) = transceiver.sender().await {
                senders.push(sender);
            }
        }
        senders
    }

    /// get_receivers returns the RTPReceivers that are currently attached to this PeerConnection
    pub async fn get_receivers(&self) -> Vec<Arc<RTPReceiver>> {
        let mut receivers = vec![];
        let rtp_transceivers = self.internal.rtp_transceivers.lock().await;
        for transceiver in &*rtp_transceivers {
            if let Some(receiver) = transceiver.receiver().await {
                receivers.push(receiver);
            }
        }
        receivers
    }

    /// get_transceivers returns the RtpTransceiver that are currently attached to this PeerConnection
    pub async fn get_transceivers(&self) -> Vec<Arc<RTPTransceiver>> {
        let rtp_transceivers = self.internal.rtp_transceivers.lock().await;
        rtp_transceivers.clone()
    }

    /// add_track adds a Track to the PeerConnection
    pub async fn add_track(
        &mut self,
        track: Arc<dyn TrackLocal + Send + Sync>,
    ) -> Result<Arc<RTPSender>> {
        if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        }

        {
            let rtp_transceivers = self.internal.rtp_transceivers.lock().await;
            for t in &*rtp_transceivers {
                if !t.stopped && t.kind == track.kind() && t.sender().await.is_none() {
                    let sender = Arc::new(RTPSender::new(
                        Arc::clone(&track),
                        Arc::clone(&self.internal.dtls_transport),
                        Arc::clone(&self.internal.media_engine),
                        self.internal.interceptor.clone(),
                    ));

                    if let Err(err) = t
                        .set_sender_track(Some(Arc::clone(&sender)), Some(Arc::clone(&track)))
                        .await
                    {
                        let _ = sender.stop().await;
                        let _ = t.set_sender(None).await;
                        return Err(err);
                    }

                    PeerConnection::do_negotiation_needed(NegotiationNeededParams {
                        on_negotiation_needed_handler: Arc::clone(
                            &self.internal.on_negotiation_needed_handler,
                        ),
                        is_closed: Arc::clone(&self.internal.is_closed),
                        ops: Arc::clone(&self.internal.ops),
                        negotiation_needed_state: Arc::clone(
                            &self.internal.negotiation_needed_state,
                        ),
                        is_negotiation_needed: Arc::clone(&self.internal.is_negotiation_needed),
                        signaling_state: Arc::clone(&self.internal.signaling_state),
                        check_negotiation_needed_params: CheckNegotiationNeededParams {
                            sctp_transport: Arc::clone(&self.internal.sctp_transport),
                            rtp_transceivers: Arc::clone(&self.internal.rtp_transceivers),
                            current_local_description: self
                                .internal
                                .current_local_description
                                .clone(),
                            current_remote_description: self
                                .internal
                                .current_remote_description
                                .clone(),
                        },
                    })
                    .await;

                    return Ok(sender);
                }
            }
        }

        let transceiver = self
            .internal
            .new_transceiver_from_track(RTPTransceiverDirection::Sendrecv, track)?;
        self.internal
            .add_rtp_transceiver(Arc::clone(&transceiver))
            .await;

        match transceiver.sender().await {
            Some(sender) => Ok(sender),
            None => Err(Error::ErrRTPSenderNil.into()),
        }
    }

    /// remove_track removes a Track from the PeerConnection
    pub async fn remove_track(&self, sender: &Arc<RTPSender>) -> Result<()> {
        if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        }

        let mut transceiver = None;
        {
            let rtp_transceivers = self.internal.rtp_transceivers.lock().await;
            for t in &*rtp_transceivers {
                if let Some(s) = t.sender().await {
                    if s.id == sender.id {
                        transceiver = Some(t.clone());
                        break;
                    }
                }
            }
        }

        if let Some(t) = transceiver {
            if sender.stop().await.is_ok() && t.set_sending_track(None).await.is_ok() {
                PeerConnection::do_negotiation_needed(NegotiationNeededParams {
                    on_negotiation_needed_handler: Arc::clone(
                        &self.internal.on_negotiation_needed_handler,
                    ),
                    is_closed: Arc::clone(&self.internal.is_closed),
                    ops: Arc::clone(&self.internal.ops),
                    negotiation_needed_state: Arc::clone(&self.internal.negotiation_needed_state),
                    is_negotiation_needed: Arc::clone(&self.internal.is_negotiation_needed),
                    signaling_state: Arc::clone(&self.internal.signaling_state),
                    check_negotiation_needed_params: CheckNegotiationNeededParams {
                        sctp_transport: Arc::clone(&self.internal.sctp_transport),
                        rtp_transceivers: Arc::clone(&self.internal.rtp_transceivers),
                        current_local_description: self.internal.current_local_description.clone(),
                        current_remote_description: self
                            .internal
                            .current_remote_description
                            .clone(),
                    },
                })
                .await;
            }
            Ok(())
        } else {
            Err(Error::ErrSenderNotCreatedByConnection.into())
        }
    }

    /// add_transceiver_from_kind Create a new RtpTransceiver and adds it to the set of transceivers.
    pub async fn add_transceiver_from_kind(
        &mut self,
        kind: RTPCodecType,
        init: &[RTPTransceiverInit],
    ) -> Result<Arc<RTPTransceiver>> {
        self.internal.add_transceiver_from_kind(kind, init).await
    }

    /// add_transceiver_from_track Create a new RtpTransceiver(SendRecv or SendOnly) and add it to the set of transceivers.
    pub async fn add_transceiver_from_track(
        &mut self,
        track: &Arc<dyn TrackLocal + Send + Sync>, //Why compiler complains if "track: Arc<dyn TrackLocal + Send + Sync>"?
        init: &[RTPTransceiverInit],
    ) -> Result<Arc<RTPTransceiver>> {
        if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        }

        let direction = match init.len() {
            0 => RTPTransceiverDirection::Sendrecv,
            1 => init[0].direction,
            _ => return Err(Error::ErrPeerConnAddTransceiverFromTrackOnlyAcceptsOne.into()),
        };

        let t = self
            .internal
            .new_transceiver_from_track(direction, Arc::clone(track))?;

        self.internal.add_rtp_transceiver(Arc::clone(&t)).await;

        Ok(t)
    }

    /// create_data_channel creates a new DataChannel object with the given label
    /// and optional DataChannelInit used to configure properties of the
    /// underlying channel such as data reliability.
    pub async fn create_data_channel(
        &self,
        label: &str,
        options: Option<DataChannelConfig>,
    ) -> Result<Arc<DataChannel>> {
        // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #2)
        if self.internal.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        }

        let mut params = DataChannelParameters {
            label: label.to_owned(),
            ordered: true,
            ..Default::default()
        };

        // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #19)
        if let Some(options) = options {
            if let Some(id) = options.id {
                params.id = id;
            }

            // Ordered indicates if data is allowed to be delivered out of order. The
            // default value of true, guarantees that data will be delivered in order.
            // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #9)
            if let Some(ordered) = options.ordered {
                params.ordered = ordered;
            }

            // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #7)
            if let Some(max_packet_life_time) = options.max_packet_life_time {
                params.max_packet_life_time = max_packet_life_time;
            }

            // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #8)
            if let Some(max_retransmits) = options.max_retransmits {
                params.max_retransmits = max_retransmits;
            }

            // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #10)
            if let Some(protocol) = options.protocol {
                params.protocol = protocol;
            }

            // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #11)
            if params.protocol.len() > 65535 {
                return Err(Error::ErrProtocolTooLarge.into());
            }

            // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #12)
            if let Some(negotiated) = options.negotiated {
                params.negotiated = negotiated;
            }
        }

        let d = Arc::new(DataChannel::new(
            params,
            Arc::clone(&self.internal.setting_engine),
        ));

        // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #16)
        if d.max_packet_lifetime != 0 && d.max_retransmits != 0 {
            return Err(Error::ErrRetransmitsOrPacketLifeTime.into());
        }

        {
            let mut data_channels = self.internal.sctp_transport.data_channels.lock().await;
            data_channels.push(Arc::clone(&d));
        }
        self.internal
            .sctp_transport
            .data_channels_requested
            .fetch_add(1, Ordering::SeqCst);

        // If SCTP already connected open all the channels
        if self.internal.sctp_transport.state() == SCTPTransportState::Connected {
            d.open(Arc::clone(&self.internal.sctp_transport)).await?;
        }

        PeerConnection::do_negotiation_needed(NegotiationNeededParams {
            on_negotiation_needed_handler: Arc::clone(&self.internal.on_negotiation_needed_handler),
            is_closed: Arc::clone(&self.internal.is_closed),
            ops: Arc::clone(&self.internal.ops),
            negotiation_needed_state: Arc::clone(&self.internal.negotiation_needed_state),
            is_negotiation_needed: Arc::clone(&self.internal.is_negotiation_needed),
            signaling_state: Arc::clone(&self.internal.signaling_state),
            check_negotiation_needed_params: CheckNegotiationNeededParams {
                sctp_transport: Arc::clone(&self.internal.sctp_transport),
                rtp_transceivers: Arc::clone(&self.internal.rtp_transceivers),
                current_local_description: self.internal.current_local_description.clone(),
                current_remote_description: self.internal.current_remote_description.clone(),
            },
        })
        .await;

        Ok(d)
    }

    /// set_identity_provider is used to configure an identity provider to generate identity assertions
    pub fn set_identity_provider(&self, _provider: &str) -> Result<()> {
        Err(Error::ErrPeerConnSetIdentityProviderNotImplemented.into())
    }

    /// write_rtcp sends a user provided RTCP packet to the connected peer. If no peer is connected the
    /// packet is discarded. It also runs any configured interceptors.
    pub async fn write_rtcp(&self, pkts: &dyn rtcp::packet::Packet) -> Result<()> {
        if let Some(rtc_writer) = &self.interceptor_rtcp_writer {
            let a = Attributes::new();
            rtc_writer.write(pkts, &a).await?;
        }
        Ok(())
    }

    /// close ends the PeerConnection
    pub async fn close(&self) -> Result<()> {
        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-close (step #1)
        if self.internal.is_closed.load(Ordering::SeqCst) {
            return Ok(());
        }

        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-close (step #2)
        self.internal.is_closed.store(true, Ordering::SeqCst);

        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-close (step #3)
        self.internal
            .signaling_state
            .store(SignalingState::Closed as u8, Ordering::SeqCst);

        // Try closing everything and collect the errors
        // Shutdown strategy:
        // 1. All Conn close by closing their underlying Conn.
        // 2. A Mux stops this chain. It won't close the underlying
        //    Conn if one of the endpoints is closed down. To
        //    continue the chain the Mux has to be closed.
        let mut close_errs = vec![];

        if let Some(interceptor) = &self.internal.interceptor {
            if let Err(err) = interceptor.close().await {
                close_errs.push(err);
            }
        }

        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-close (step #4)
        {
            let rtp_transceivers = self.internal.rtp_transceivers.lock().await;
            for t in &*rtp_transceivers {
                if !t.stopped {
                    if let Err(err) = t.stop().await {
                        close_errs.push(err);
                    }
                }
            }
        }

        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-close (step #5)
        {
            let data_channels = self.internal.sctp_transport.data_channels.lock().await;
            for d in &*data_channels {
                d.set_ready_state(DataChannelState::Closed);
            }
        }

        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-close (step #6)
        if let Err(err) = self.internal.sctp_transport.stop().await {
            close_errs.push(err);
        }

        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-close (step #7)
        if let Err(err) = self.internal.dtls_transport.stop().await {
            close_errs.push(err);
        }

        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-close (step #8, #9, #10)
        if let Err(err) = self.internal.ice_transport.stop().await {
            close_errs.push(err);
        }

        // https://www.w3.org/TR/webrtc/#dom-rtcpeerconnection-close (step #11)
        PeerConnection::update_connection_state(
            &self.internal.on_peer_connection_state_change_handler,
            &self.internal.is_closed,
            &self.internal.peer_connection_state,
            self.ice_connection_state(),
            self.internal.dtls_transport.state(),
        )
        .await;

        if let Err(err) = self.internal.ops.close().await {
            close_errs.push(err);
        }

        flatten_errs(close_errs)
    }

    /// CurrentLocalDescription represents the local description that was
    /// successfully negotiated the last time the PeerConnection transitioned
    /// into the stable state plus any local candidates that have been generated
    /// by the ICEAgent since the offer or answer was created.
    pub async fn current_local_description(&self) -> Option<SessionDescription> {
        let local_description = {
            let current_local_description = self.internal.current_local_description.lock().await;
            current_local_description.clone()
        };
        let ice_gather = Some(&self.internal.ice_gatherer);
        let ice_gathering_state = self.ice_gathering_state();

        populate_local_candidates(local_description.as_ref(), ice_gather, ice_gathering_state).await
    }

    /// PendingLocalDescription represents a local description that is in the
    /// process of being negotiated plus any local candidates that have been
    /// generated by the ICEAgent since the offer or answer was created. If the
    /// PeerConnection is in the stable state, the value is null.
    pub async fn pending_local_description(&self) -> Option<SessionDescription> {
        let local_description = {
            let pending_local_description = self.internal.pending_local_description.lock().await;
            pending_local_description.clone()
        };
        let ice_gather = Some(&self.internal.ice_gatherer);
        let ice_gathering_state = self.ice_gathering_state();

        populate_local_candidates(local_description.as_ref(), ice_gather, ice_gathering_state).await
    }

    /// current_remote_description represents the last remote description that was
    /// successfully negotiated the last time the PeerConnection transitioned
    /// into the stable state plus any remote candidates that have been supplied
    /// via add_icecandidate() since the offer or answer was created.
    pub async fn current_remote_description(&self) -> Option<SessionDescription> {
        let current_remote_description = self.internal.current_remote_description.lock().await;
        current_remote_description.clone()
    }

    /// pending_remote_description represents a remote description that is in the
    /// process of being negotiated, complete with any remote candidates that
    /// have been supplied via add_icecandidate() since the offer or answer was
    /// created. If the PeerConnection is in the stable state, the value is
    /// null.
    pub async fn pending_remote_description(&self) -> Option<SessionDescription> {
        let pending_remote_description = self.internal.pending_remote_description.lock().await;
        pending_remote_description.clone()
    }

    /// signaling_state attribute returns the signaling state of the
    /// PeerConnection instance.
    pub fn signaling_state(&self) -> SignalingState {
        self.internal.signaling_state.load(Ordering::SeqCst).into()
    }

    /// icegathering_state attribute returns the ICE gathering state of the
    /// PeerConnection instance.
    pub fn ice_gathering_state(&self) -> ICEGatheringState {
        self.internal.ice_gathering_state()
    }

    /// connection_state attribute returns the connection state of the
    /// PeerConnection instance.
    pub fn connection_state(&self) -> PeerConnectionState {
        self.internal
            .peer_connection_state
            .load(Ordering::SeqCst)
            .into()
    }

    // GetStats return data providing statistics about the overall connection
    /*TODO: func (pc *PeerConnection) GetStats() StatsReport {
        var (
            dataChannelsAccepted  uint32
            dataChannelsClosed    uint32
            dataChannelsOpened    uint32
            dataChannelsRequested uint32
        )
        statsCollector := newStatsReportCollector()
        statsCollector.Collecting()

        self.mu.Lock()
        if self.iceGatherer != nil {
            self.iceGatherer.collectStats(statsCollector)
        }
        if self.iceTransport != nil {
            self.iceTransport.collectStats(statsCollector)
        }

        self.sctpTransport.lock.Lock()
        dataChannels := append([]*DataChannel{}, self.sctpTransport.dataChannels...)
        dataChannelsAccepted = self.sctpTransport.dataChannelsAccepted
        dataChannelsOpened = self.sctpTransport.dataChannelsOpened
        dataChannelsRequested = self.sctpTransport.dataChannelsRequested
        self.sctpTransport.lock.Unlock()

        for _, d := range dataChannels {
            state := d.ReadyState()
            if state != DataChannelStateConnecting && state != DataChannelStateOpen {
                dataChannelsClosed++
            }

            d.collectStats(statsCollector)
        }
        self.sctpTransport.collectStats(statsCollector)

        stats := PeerConnectionStats{
            Timestamp:             statsTimestampNow(),
            Type:                  StatsTypePeerConnection,
            ID:                    self.stats_id,
            DataChannelsAccepted:  dataChannelsAccepted,
            DataChannelsClosed:    dataChannelsClosed,
            DataChannelsOpened:    dataChannelsOpened,
            DataChannelsRequested: dataChannelsRequested,
        }

        statsCollector.Collect(stats.ID, stats)

        certificates := self.configuration.Certificates
        for _, certificate := range certificates {
            if err := certificate.collectStats(statsCollector); err != nil {
                continue
            }
        }
        self.mu.Unlock()

        self.api.mediaEngine.collectStats(statsCollector)

        return statsCollector.Ready()
    }
    */

    /// sctp returns the SCTPTransport for this PeerConnection
    ///
    /// The SCTP transport over which SCTP data is sent and received. If SCTP has not been negotiated, the value is nil.
    /// https://www.w3.org/TR/webrtc/#attributes-15
    pub fn sctp(&self) -> Arc<SCTPTransport> {
        Arc::clone(&self.internal.sctp_transport)
    }

    /// gathering_complete_promise is a Pion specific helper function that returns a channel that is closed when gathering is complete.
    /// This function may be helpful in cases where you are unable to trickle your ICE Candidates.
    ///
    /// It is better to not use this function, and instead trickle candidates. If you use this function you will see longer connection startup times.
    /// When the call is connected you will see no impact however.
    pub async fn gathering_complete_promise(&self) -> mpsc::Receiver<()> {
        let (gathering_complete_tx, gathering_complete_rx) = mpsc::channel(1);

        // It's possible to miss the GatherComplete event since setGatherCompleteHandler is an atomic operation and the
        // promise might have been created after the gathering is finished. Therefore, we need to check if the ICE gathering
        // state has changed to complete so that we don't block the caller forever.
        let done = Arc::new(Mutex::new(Some(gathering_complete_tx)));
        let done2 = Arc::clone(&done);
        self.internal
            .set_gather_complete_handler(Box::new(move || {
                log::trace!("setGatherCompleteHandler");
                let done3 = Arc::clone(&done2);
                Box::pin(async move {
                    let mut d = done3.lock().await;
                    d.take();
                })
            }))
            .await;

        if self.ice_gathering_state() == ICEGatheringState::Complete {
            log::trace!("ICEGatheringState::Complete");
            let mut d = done.lock().await;
            d.take();
        }

        gathering_complete_rx
    }
}
