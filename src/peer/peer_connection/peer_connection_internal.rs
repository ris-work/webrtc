use super::*;
use std::sync::atomic::AtomicIsize;

pub(crate) struct PeerConnectionInternal {
    /// a value containing the last known greater mid value
    /// we internally generate mids as numbers. Needed since JSEP
    /// requires that when reusing a media section a new unique mid
    /// should be defined (see JSEP 3.4.1).
    pub(super) greater_mid: AtomicIsize,
    pub(super) sdp_origin: Mutex<sdp::session_description::Origin>,
    pub(super) last_offer: Mutex<String>,
    pub(super) last_answer: Mutex<String>,

    pub(super) on_negotiation_needed_handler: Arc<Mutex<Option<OnNegotiationNeededHdlrFn>>>,
    pub(super) is_closed: Arc<AtomicBool>,

    /// ops is an operations queue which will ensure the enqueued actions are
    /// executed in order. It is used for asynchronously, but serially processing
    /// remote and local descriptions
    pub(super) ops: Arc<Operations>,
    pub(super) negotiation_needed_state: Arc<AtomicU8>,
    pub(super) is_negotiation_needed: Arc<AtomicBool>,
    pub(super) signaling_state: Arc<AtomicU8>,

    pub(super) ice_transport: Arc<ICETransport>,
    pub(super) dtls_transport: Arc<DTLSTransport>,
    pub(super) on_peer_connection_state_change_handler:
        Arc<Mutex<Option<OnPeerConnectionStateChangeHdlrFn>>>,
    pub(super) peer_connection_state: Arc<AtomicU8>,
    pub(super) ice_connection_state: Arc<AtomicU8>,

    pub(super) sctp_transport: Arc<SCTPTransport>,
    pub(super) rtp_transceivers: Arc<Mutex<Vec<Arc<RTPTransceiver>>>>,

    pub(super) on_track_handler: Arc<Mutex<Option<OnTrackHdlrFn>>>,
    pub(super) on_signaling_state_change_handler: Arc<Mutex<Option<OnSignalingStateChangeHdlrFn>>>,
    pub(super) on_ice_connection_state_change_handler:
        Arc<Mutex<Option<OnICEConnectionStateChangeHdlrFn>>>,
    pub(super) on_data_channel_handler: Arc<Mutex<Option<OnDataChannelHdlrFn>>>,

    pub(super) ice_gatherer: Arc<ICEGatherer>,

    pub(super) current_local_description: Arc<Mutex<Option<SessionDescription>>>,
    pub(super) current_remote_description: Arc<Mutex<Option<SessionDescription>>>,
    pub(super) pending_local_description: Arc<Mutex<Option<SessionDescription>>>,
    pub(super) pending_remote_description: Arc<Mutex<Option<SessionDescription>>>,

    // A reference to the associated API state used by this connection
    pub(super) setting_engine: Arc<SettingEngine>,
    pub(crate) media_engine: Arc<MediaEngine>,
    pub(super) interceptor: Arc<dyn Interceptor + Send + Sync>,
}

impl PeerConnectionInternal {
    pub(super) async fn new(api: &API, configuration: &mut RTCConfiguration) -> Result<Self> {
        let mut pc = PeerConnectionInternal {
            greater_mid: AtomicIsize::new(-1),
            sdp_origin: Mutex::new(Default::default()),
            last_offer: Mutex::new("".to_owned()),
            last_answer: Mutex::new("".to_owned()),

            on_negotiation_needed_handler: Arc::new(Default::default()),
            ops: Arc::new(Operations::new()),
            is_closed: Arc::new(AtomicBool::new(false)),
            is_negotiation_needed: Arc::new(AtomicBool::new(false)),
            negotiation_needed_state: Arc::new(AtomicU8::new(NegotiationNeededState::Empty as u8)),
            signaling_state: Arc::new(AtomicU8::new(RTCSignalingState::Stable as u8)),
            ice_transport: Arc::new(Default::default()),
            dtls_transport: Arc::new(Default::default()),
            ice_connection_state: Arc::new(AtomicU8::new(ICEConnectionState::New as u8)),
            sctp_transport: Arc::new(Default::default()),
            rtp_transceivers: Arc::new(Default::default()),
            on_track_handler: Arc::new(Default::default()),
            on_signaling_state_change_handler: Arc::new(Default::default()),
            on_ice_connection_state_change_handler: Arc::new(Default::default()),
            on_data_channel_handler: Arc::new(Default::default()),
            ice_gatherer: Arc::new(Default::default()),
            current_local_description: Arc::new(Default::default()),
            current_remote_description: Arc::new(Default::default()),
            pending_local_description: Arc::new(Default::default()),
            peer_connection_state: Arc::new(AtomicU8::new(PeerConnectionState::New as u8)),

            setting_engine: Arc::clone(&api.setting_engine),
            media_engine: if !api.setting_engine.disable_media_engine_copy {
                Arc::new(api.media_engine.clone_to())
            } else {
                Arc::clone(&api.media_engine)
            },
            interceptor: Arc::clone(&api.interceptor),
            on_peer_connection_state_change_handler: Arc::new(Default::default()),
            pending_remote_description: Arc::new(Default::default()),
        };

        // Create the ice gatherer
        pc.ice_gatherer = Arc::new(api.new_ice_gatherer(ICEGatherOptions {
            ice_servers: configuration.get_ice_servers(),
            ice_gather_policy: configuration.ice_transport_policy,
        })?);

        // Create the ice transport
        pc.ice_transport = pc.create_ice_transport(api).await;

        // Create the DTLS transport
        let certificates = configuration.certificates.drain(..).collect();
        pc.dtls_transport =
            Arc::new(api.new_dtls_transport(Arc::clone(&pc.ice_transport), certificates)?);

        // Create the SCTP transport
        pc.sctp_transport = Arc::new(api.new_sctp_transport(Arc::clone(&pc.dtls_transport))?);

        // Wire up the on datachannel handler
        let on_data_channel_handler = Arc::clone(&pc.on_data_channel_handler);
        pc.sctp_transport
            .on_data_channel(Box::new(move |d: Arc<DataChannel>| {
                let on_data_channel_handler2 = Arc::clone(&on_data_channel_handler);
                Box::pin(async move {
                    let mut handler = on_data_channel_handler2.lock().await;
                    if let Some(f) = &mut *handler {
                        f(d).await;
                    }
                })
            }))
            .await;

        Ok(pc)
    }

    pub(super) async fn start_rtp(
        self: &Arc<Self>,
        is_renegotiation: bool,
        remote_desc: Arc<SessionDescription>,
        sdp_semantics: SDPSemantics,
    ) -> Result<()> {
        let mut track_details = if let Some(parsed) = &remote_desc.parsed {
            track_details_from_sdp(parsed)
        } else {
            vec![]
        };

        let current_transceivers = {
            let current_transceivers = self.rtp_transceivers.lock().await;
            current_transceivers.clone()
        };

        if is_renegotiation {
            for t in &current_transceivers {
                if let Some(receiver) = t.receiver().await {
                    if let Some(track) = receiver.track().await {
                        let ssrc = track.ssrc();
                        if let Some(details) = track_details_for_ssrc(&track_details, ssrc) {
                            track.set_id(details.id.clone()).await;
                            track.set_stream_id(details.stream_id.clone()).await;
                            continue;
                        }
                    }

                    if let Err(err) = receiver.stop().await {
                        log::warn!("Failed to stop RtpReceiver: {}", err);
                        continue;
                    }

                    let receiver = Arc::new(RTPReceiver::new(
                        receiver.kind(),
                        Arc::clone(&self.dtls_transport),
                        Arc::clone(&self.media_engine),
                        Arc::clone(&self.interceptor),
                    ));
                    t.set_receiver(Some(receiver)).await;
                }
            }
        }

        self.start_rtp_receivers(&mut track_details, &current_transceivers, sdp_semantics)
            .await?;

        if let Some(parsed) = &remote_desc.parsed {
            if have_application_media_section(parsed) {
                self.start_sctp().await;
            }
        }

        if !is_renegotiation {
            self.undeclared_media_processor()
        }

        Ok(())
    }

    /// undeclared_media_processor handles RTP/RTCP packets that don't match any a:ssrc lines
    fn undeclared_media_processor(self: &Arc<Self>) {
        let dtls_transport = Arc::clone(&self.dtls_transport);
        let is_closed = Arc::clone(&self.is_closed);
        let pci = Arc::clone(self);
        tokio::spawn(async move {
            let simulcast_routine_count = Arc::new(AtomicU64::new(0));
            loop {
                let srtp_session = match dtls_transport.get_srtp_session().await {
                    Some(s) => s,
                    None => {
                        log::warn!("undeclared_media_processor failed to open SrtpSession");
                        return;
                    }
                };

                let stream = match srtp_session.accept().await {
                    Ok(stream) => Arc::new(stream),
                    Err(err) => {
                        log::warn!("Failed to accept RTP {}", err);
                        return;
                    }
                };

                if is_closed.load(Ordering::SeqCst) {
                    if let Err(err) = stream.close().await {
                        log::warn!("Failed to close RTP stream {}", err);
                    }
                    continue;
                }

                if simulcast_routine_count.fetch_add(1, Ordering::SeqCst) + 1
                    >= SIMULCAST_MAX_PROBE_ROUTINES
                {
                    simulcast_routine_count.fetch_sub(1, Ordering::SeqCst);
                    log::warn!("{:?}", Error::ErrSimulcastProbeOverflow);
                    continue;
                }

                let dtls_transport2 = Arc::clone(&dtls_transport);
                let simulcast_routine_count2 = Arc::clone(&simulcast_routine_count);
                let pci2 = Arc::clone(&pci);
                tokio::spawn(async move {
                    dtls_transport2
                        .store_simulcast_stream(Arc::clone(&stream))
                        .await;

                    let ssrc = stream.get_ssrc();
                    if let Err(err) = pci2.handle_undeclared_ssrc(stream, ssrc).await {
                        log::error!(
                            "Incoming unhandled RTP ssrc({}), on_track will not be fired. {}",
                            ssrc,
                            err
                        );
                    }

                    simulcast_routine_count2.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });

        let dtls_transport = Arc::clone(&self.dtls_transport);
        tokio::spawn(async move {
            loop {
                let srtcp_session = match dtls_transport.get_srtcp_session().await {
                    Some(s) => s,
                    None => {
                        log::warn!("undeclared_media_processor failed to open SrtcpSession");
                        return;
                    }
                };

                let stream = match srtcp_session.accept().await {
                    Ok(stream) => stream,
                    Err(err) => {
                        log::warn!("Failed to accept RTCP {}", err);
                        return;
                    }
                };
                log::warn!(
                    "Incoming unhandled RTCP ssrc({}), on_track will not be fired",
                    stream.get_ssrc()
                );
            }
        });
    }

    /// start_rtp_receivers opens knows inbound SRTP streams from the remote_description
    async fn start_rtp_receivers(
        self: &Arc<Self>,
        incoming_tracks: &mut Vec<TrackDetails>,
        local_transceivers: &[Arc<RTPTransceiver>],
        sdp_semantics: SDPSemantics,
    ) -> Result<()> {
        let remote_is_plan_b = match sdp_semantics {
            SDPSemantics::PlanB => true,
            SDPSemantics::UnifiedPlanWithFallback => {
                description_is_plan_b(self.remote_description().await.as_ref())?
            }
            _ => false,
        };

        // Ensure we haven't already started a transceiver for this ssrc
        let ssrcs: Vec<SSRC> = incoming_tracks.iter().map(|x| x.ssrc).collect();
        for ssrc in ssrcs {
            for t in local_transceivers {
                if let Some(receiver) = t.receiver().await {
                    if let Some(track) = receiver.track().await {
                        if track.ssrc() != ssrc {
                            continue;
                        }
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }

                filter_track_with_ssrc(incoming_tracks, ssrc);
            }
        }

        let mut unhandled_tracks = vec![]; // incoming_tracks[:0]
        for incoming_track in incoming_tracks.iter() {
            let mut track_handled = false;
            for t in local_transceivers {
                if t.mid().await != incoming_track.mid {
                    continue;
                }

                if (incoming_track.kind != t.kind())
                    || (t.direction() != RTPTransceiverDirection::Recvonly
                        && t.direction() != RTPTransceiverDirection::Sendrecv)
                {
                    continue;
                }

                if let Some(receiver) = t.receiver().await {
                    if receiver.have_received().await {
                        continue;
                    }
                    PeerConnectionInternal::start_receiver(
                        incoming_track,
                        receiver,
                        Arc::clone(&self.media_engine),
                        Arc::clone(&self.on_track_handler),
                    )
                    .await;
                    track_handled = true;
                }
            }

            if !track_handled {
                unhandled_tracks.push(incoming_track);
            }
        }

        if remote_is_plan_b {
            for incoming in unhandled_tracks {
                let t = match self
                    .add_transceiver_from_kind(
                        incoming.kind,
                        &[RTPTransceiverInit {
                            direction: RTPTransceiverDirection::Sendrecv,
                            send_encodings: vec![],
                        }],
                    )
                    .await
                {
                    Ok(t) => t,
                    Err(err) => {
                        log::warn!(
                            "Could not add transceiver for remote SSRC {}: {}",
                            incoming.ssrc,
                            err
                        );
                        continue;
                    }
                };
                if let Some(receiver) = t.receiver().await {
                    PeerConnectionInternal::start_receiver(
                        incoming,
                        receiver,
                        Arc::clone(&self.media_engine),
                        Arc::clone(&self.on_track_handler),
                    )
                    .await;
                }
            }
        }

        Ok(())
    }

    /// Start SCTP subsystem
    async fn start_sctp(&self) {
        // Start sctp
        if let Err(err) = self
            .sctp_transport
            .start(SCTPTransportCapabilities {
                max_message_size: 0,
            })
            .await
        {
            log::warn!("Failed to start SCTP: {}", err);
            if let Err(err) = self.sctp_transport.stop().await {
                log::warn!("Failed to stop SCTPTransport: {}", err);
            }

            return;
        }

        // DataChannels that need to be opened now that SCTP is available
        // make a copy we may have incoming DataChannels mutating this while we open
        let data_channels = {
            let data_channels = self.sctp_transport.data_channels.lock().await;
            data_channels.clone()
        };

        let mut opened_dc_count = 0;
        for d in data_channels {
            if d.ready_state() == DataChannelState::Connecting {
                if let Err(err) = d.open(Arc::clone(&self.sctp_transport)).await {
                    log::warn!("failed to open data channel: {}", err);
                    continue;
                }
                opened_dc_count += 1;
            }
        }

        self.sctp_transport
            .data_channels_opened
            .fetch_add(opened_dc_count, Ordering::SeqCst);
    }

    pub(super) async fn add_transceiver_from_kind(
        &self,
        kind: RTPCodecType,
        init: &[RTPTransceiverInit],
    ) -> Result<Arc<RTPTransceiver>> {
        if self.is_closed.load(Ordering::SeqCst) {
            return Err(Error::ErrConnectionClosed.into());
        }

        let direction = match init.len() {
            0 => RTPTransceiverDirection::Sendrecv,
            1 => init[0].direction,
            _ => return Err(Error::ErrPeerConnAddTransceiverFromKindOnlyAcceptsOne.into()),
        };

        let t = match direction {
            RTPTransceiverDirection::Sendonly | RTPTransceiverDirection::Sendrecv => {
                let codecs = self.media_engine.get_codecs_by_kind(kind).await;
                if codecs.is_empty() {
                    return Err(Error::ErrNoCodecsAvailable.into());
                }
                let track = Arc::new(TrackLocalStaticSample::new(
                    codecs[0].capability.clone(),
                    math_rand_alpha(16),
                    math_rand_alpha(16),
                ));

                self.new_transceiver_from_track(direction, track).await?
            }
            RTPTransceiverDirection::Recvonly => {
                let receiver = Arc::new(RTPReceiver::new(
                    kind,
                    Arc::clone(&self.dtls_transport),
                    Arc::clone(&self.media_engine),
                    Arc::clone(&self.interceptor),
                ));

                RTPTransceiver::new(
                    Some(receiver),
                    None,
                    RTPTransceiverDirection::Recvonly,
                    kind,
                    vec![],
                    Arc::clone(&self.media_engine),
                )
                .await
            }
            _ => return Err(Error::ErrPeerConnAddTransceiverFromKindSupport.into()),
        };

        self.add_rtp_transceiver(Arc::clone(&t)).await;

        Ok(t)
    }

    pub(super) async fn new_transceiver_from_track(
        &self,
        direction: RTPTransceiverDirection,
        track: Arc<dyn TrackLocal + Send + Sync>,
    ) -> Result<Arc<RTPTransceiver>> {
        let (r, s) = match direction {
            RTPTransceiverDirection::Sendrecv => {
                let r = Some(Arc::new(RTPReceiver::new(
                    track.kind(),
                    Arc::clone(&self.dtls_transport),
                    Arc::clone(&self.media_engine),
                    Arc::clone(&self.interceptor),
                )));
                let s = Some(Arc::new(
                    RTPSender::new(
                        Arc::clone(&track),
                        Arc::clone(&self.dtls_transport),
                        Arc::clone(&self.media_engine),
                        Arc::clone(&self.interceptor),
                    )
                    .await,
                ));
                (r, s)
            }
            RTPTransceiverDirection::Sendonly => {
                let s = Some(Arc::new(
                    RTPSender::new(
                        Arc::clone(&track),
                        Arc::clone(&self.dtls_transport),
                        Arc::clone(&self.media_engine),
                        Arc::clone(&self.interceptor),
                    )
                    .await,
                ));
                (None, s)
            }
            _ => return Err(Error::ErrPeerConnAddTransceiverFromTrackSupport.into()),
        };

        Ok(RTPTransceiver::new(
            r,
            s,
            direction,
            track.kind(),
            vec![],
            Arc::clone(&self.media_engine),
        )
        .await)
    }

    /// add_rtp_transceiver appends t into rtp_transceivers
    /// and fires onNegotiationNeeded;
    /// caller of this method should hold `self.mu` lock
    pub(super) async fn add_rtp_transceiver(&self, t: Arc<RTPTransceiver>) {
        {
            let mut rtp_transceivers = self.rtp_transceivers.lock().await;
            rtp_transceivers.push(t);
        }
        PeerConnection::do_negotiation_needed(NegotiationNeededParams {
            on_negotiation_needed_handler: Arc::clone(&self.on_negotiation_needed_handler),
            is_closed: Arc::clone(&self.is_closed),
            ops: Arc::clone(&self.ops),
            negotiation_needed_state: Arc::clone(&self.negotiation_needed_state),
            is_negotiation_needed: Arc::clone(&self.is_negotiation_needed),
            signaling_state: Arc::clone(&self.signaling_state),
            check_negotiation_needed_params: CheckNegotiationNeededParams {
                sctp_transport: Arc::clone(&self.sctp_transport),
                rtp_transceivers: Arc::clone(&self.rtp_transceivers),
                current_local_description: Arc::clone(&self.current_local_description),
                current_remote_description: Arc::clone(&self.current_remote_description),
            },
        })
        .await;
    }

    pub(super) async fn remote_description(self: &Arc<Self>) -> Option<SessionDescription> {
        let pending_remote_description = self.pending_remote_description.lock().await;
        if pending_remote_description.is_some() {
            pending_remote_description.clone()
        } else {
            let current_remote_description = self.current_remote_description.lock().await;
            current_remote_description.clone()
        }
    }

    pub(super) async fn set_gather_complete_handler(&self, f: OnGatheringCompleteHdlrFn) {
        self.ice_gatherer.on_gathering_complete(f).await;
    }

    /// Start all transports. PeerConnection now has enough state
    pub(super) async fn start_transports(
        self: &Arc<Self>,
        ice_role: ICERole,
        dtls_role: DTLSRole,
        remote_ufrag: String,
        remote_pwd: String,
        fingerprint: String,
        fingerprint_hash: String,
    ) {
        // Start the ice transport
        if let Err(err) = self
            .ice_transport
            .start(
                &ICEParameters {
                    username_fragment: remote_ufrag,
                    password: remote_pwd,
                    ice_lite: false,
                },
                Some(ice_role),
            )
            .await
        {
            log::warn!("Failed to start manager ice: {}", err);
            return;
        }

        // Start the dtls_transport transport
        let result = self
            .dtls_transport
            .start(DTLSParameters {
                role: dtls_role,
                fingerprints: vec![DTLSFingerprint {
                    algorithm: fingerprint_hash,
                    value: fingerprint,
                }],
            })
            .await;
        PeerConnection::update_connection_state(
            &self.on_peer_connection_state_change_handler,
            &self.is_closed,
            &self.peer_connection_state,
            self.ice_connection_state.load(Ordering::SeqCst).into(),
            self.dtls_transport.state(),
        )
        .await;
        if let Err(err) = result {
            log::warn!("Failed to start manager dtls: {}", err);
        }
    }

    /// generate_unmatched_sdp generates an SDP that doesn't take remote state into account
    /// This is used for the initial call for CreateOffer
    pub(super) async fn generate_unmatched_sdp(
        &self,
        local_transceivers: Vec<Arc<RTPTransceiver>>,
        use_identity: bool,
        sdp_semantics: SDPSemantics,
    ) -> Result<sdp::session_description::SessionDescription> {
        let d = sdp::session_description::SessionDescription::new_jsep_session_description(
            use_identity,
        );

        let ice_params = self.ice_gatherer.get_local_parameters().await?;

        let candidates = self.ice_gatherer.get_local_candidates().await?;

        let is_plan_b = sdp_semantics == SDPSemantics::PlanB;
        let mut media_sections = vec![];

        // Needed for self.sctpTransport.dataChannelsRequested
        if is_plan_b {
            let mut video = vec![];
            let mut audio = vec![];

            for t in &local_transceivers {
                if t.kind == RTPCodecType::Video {
                    video.push(Arc::clone(t));
                } else if t.kind == RTPCodecType::Audio {
                    audio.push(Arc::clone(t));
                }
                if let Some(sender) = t.sender().await {
                    sender.set_negotiated();
                }
            }

            if !video.is_empty() {
                media_sections.push(MediaSection {
                    id: "video".to_owned(),
                    transceivers: video,
                    ..Default::default()
                })
            }
            if !audio.is_empty() {
                media_sections.push(MediaSection {
                    id: "audio".to_owned(),
                    transceivers: audio,
                    ..Default::default()
                });
            }

            if self
                .sctp_transport
                .data_channels_requested
                .load(Ordering::SeqCst)
                != 0
            {
                media_sections.push(MediaSection {
                    id: "data".to_owned(),
                    data: true,
                    ..Default::default()
                });
            }
        } else {
            {
                for t in &local_transceivers {
                    if let Some(sender) = t.sender().await {
                        sender.set_negotiated();
                    }
                    media_sections.push(MediaSection {
                        id: t.mid().await,
                        transceivers: vec![Arc::clone(t)],
                        ..Default::default()
                    });
                }
            }

            if self
                .sctp_transport
                .data_channels_requested
                .load(Ordering::SeqCst)
                != 0
            {
                media_sections.push(MediaSection {
                    id: format!("{}", media_sections.len()),
                    data: true,
                    ..Default::default()
                });
            }
        }

        let dtls_fingerprints = if let Some(cert) = self.dtls_transport.certificates.first() {
            cert.get_fingerprints()?
        } else {
            return Err(Error::ErrNonCertificate.into());
        };

        let params = PopulateSdpParams {
            is_plan_b,
            media_description_fingerprint: self.setting_engine.sdp_media_level_fingerprints,
            is_icelite: self.setting_engine.candidates.ice_lite,
            connection_role: DEFAULT_DTLS_ROLE_OFFER.to_connection_role(),
            ice_gathering_state: self.ice_gathering_state(),
        };
        populate_sdp(
            d,
            &dtls_fingerprints,
            &self.media_engine,
            &candidates,
            &ice_params,
            &media_sections,
            params,
        )
        .await
    }

    /// generate_matched_sdp generates a SDP and takes the remote state into account
    /// this is used everytime we have a remote_description
    pub(super) async fn generate_matched_sdp(
        &self,
        mut local_transceivers: Vec<Arc<RTPTransceiver>>,
        use_identity: bool,
        include_unmatched: bool,
        connection_role: ConnectionRole,
        sdp_semantics: SDPSemantics,
    ) -> Result<sdp::session_description::SessionDescription> {
        let d = sdp::session_description::SessionDescription::new_jsep_session_description(
            use_identity,
        );

        let ice_params = self.ice_gatherer.get_local_parameters().await?;
        let candidates = self.ice_gatherer.get_local_candidates().await?;

        let remote_description = {
            let pending_remote_description = self.pending_remote_description.lock().await;
            if pending_remote_description.is_some() {
                pending_remote_description.clone()
            } else {
                let current_remote_description = self.current_remote_description.lock().await;
                current_remote_description.clone()
            }
        };

        let detected_plan_b = description_is_plan_b(remote_description.as_ref())?;
        let mut media_sections = vec![];
        let mut already_have_application_media_section = false;
        if let Some(remote_description) = remote_description.as_ref() {
            if let Some(parsed) = &remote_description.parsed {
                for media in &parsed.media_descriptions {
                    if let Some(mid_value) = get_mid_value(media) {
                        if mid_value.is_empty() {
                            return Err(Error::ErrPeerConnRemoteDescriptionWithoutMidValue.into());
                        }

                        if media.media_name.media == MEDIA_SECTION_APPLICATION {
                            media_sections.push(MediaSection {
                                id: mid_value.to_owned(),
                                data: true,
                                ..Default::default()
                            });
                            already_have_application_media_section = true;
                            continue;
                        }

                        let kind = RTPCodecType::from(media.media_name.media.as_str());
                        let direction = get_peer_direction(media);
                        if kind == RTPCodecType::Unspecified
                            || direction == RTPTransceiverDirection::Unspecified
                        {
                            continue;
                        }

                        if sdp_semantics == SDPSemantics::PlanB
                            || (sdp_semantics == SDPSemantics::UnifiedPlanWithFallback
                                && detected_plan_b)
                        {
                            if !detected_plan_b {
                                return Err(Error::ErrIncorrectSDPSemantics.into());
                            }
                            // If we're responding to a plan-b offer, then we should try to fill up this
                            // media entry with all matching local transceivers
                            let mut media_transceivers = vec![];
                            loop {
                                // keep going until we can't get any more
                                if let Some(t) = satisfy_type_and_direction(
                                    kind,
                                    direction,
                                    &mut local_transceivers,
                                )
                                .await
                                {
                                    if let Some(sender) = t.sender().await {
                                        sender.set_negotiated();
                                    }
                                    media_transceivers.push(t);
                                } else {
                                    if media_transceivers.is_empty() {
                                        let t = RTPTransceiver::new(
                                            None,
                                            None,
                                            RTPTransceiverDirection::Inactive,
                                            kind,
                                            vec![],
                                            Arc::clone(&self.media_engine),
                                        )
                                        .await;
                                        media_transceivers.push(t);
                                    }
                                    break;
                                }
                            }
                            media_sections.push(MediaSection {
                                id: mid_value.to_owned(),
                                transceivers: media_transceivers,
                                ..Default::default()
                            });
                        } else if sdp_semantics == SDPSemantics::UnifiedPlan
                            || sdp_semantics == SDPSemantics::UnifiedPlanWithFallback
                        {
                            if detected_plan_b {
                                return Err(Error::ErrIncorrectSDPSemantics.into());
                            }
                            if let Some(t) = find_by_mid(mid_value, &mut local_transceivers).await {
                                if let Some(sender) = t.sender().await {
                                    sender.set_negotiated();
                                }
                                let media_transceivers = vec![t];
                                media_sections.push(MediaSection {
                                    id: mid_value.to_owned(),
                                    transceivers: media_transceivers,
                                    rid_map: get_rids(media),
                                    ..Default::default()
                                });
                            } else {
                                return Err(Error::ErrPeerConnTranscieverMidNil.into());
                            }
                        }
                    } else {
                        return Err(Error::ErrPeerConnRemoteDescriptionWithoutMidValue.into());
                    }
                }
            }
        }

        // If we are offering also include unmatched local transceivers
        if include_unmatched {
            if !detected_plan_b {
                for t in &local_transceivers {
                    if let Some(sender) = t.sender().await {
                        sender.set_negotiated();
                    }
                    media_sections.push(MediaSection {
                        id: t.mid().await,
                        transceivers: vec![Arc::clone(t)],
                        ..Default::default()
                    });
                }
            }

            if self
                .sctp_transport
                .data_channels_requested
                .load(Ordering::SeqCst)
                != 0
                && !already_have_application_media_section
            {
                if detected_plan_b {
                    media_sections.push(MediaSection {
                        id: "data".to_owned(),
                        data: true,
                        ..Default::default()
                    });
                } else {
                    media_sections.push(MediaSection {
                        id: format!("{}", media_sections.len()),
                        data: true,
                        ..Default::default()
                    });
                }
            }
        }

        if sdp_semantics == SDPSemantics::UnifiedPlanWithFallback && detected_plan_b {
            log::info!("Plan-B Offer detected; responding with Plan-B Answer");
        }

        let dtls_fingerprints = if let Some(cert) = self.dtls_transport.certificates.first() {
            cert.get_fingerprints()?
        } else {
            return Err(Error::ErrNonCertificate.into());
        };

        let params = PopulateSdpParams {
            is_plan_b: detected_plan_b,
            media_description_fingerprint: self.setting_engine.sdp_media_level_fingerprints,
            is_icelite: self.setting_engine.candidates.ice_lite,
            connection_role,
            ice_gathering_state: self.ice_gathering_state(),
        };
        populate_sdp(
            d,
            &dtls_fingerprints,
            &self.media_engine,
            &candidates,
            &ice_params,
            &media_sections,
            params,
        )
        .await
    }

    pub(super) fn ice_gathering_state(&self) -> RTCIceGatheringState {
        match self.ice_gatherer.state() {
            RTCIceGathererState::New => RTCIceGatheringState::New,
            RTCIceGathererState::Gathering => RTCIceGatheringState::Gathering,
            _ => RTCIceGatheringState::Complete,
        }
    }

    async fn handle_undeclared_ssrc(
        self: &Arc<Self>,
        rtp_stream: Arc<Stream>,
        ssrc: SSRC,
    ) -> Result<()> {
        if let Some(rd) = self.remote_description().await {
            if let Some(parsed) = &rd.parsed {
                // If the remote SDP was only one media section the ssrc doesn't have to be explicitly declared
                if parsed.media_descriptions.len() == 1 {
                    if let Some(only_media_section) = parsed.media_descriptions.first() {
                        for a in &only_media_section.attributes {
                            if a.key == SSRC_STR {
                                return Err(
                                    Error::ErrPeerConnSingleMediaSectionHasExplicitSSRC.into()
                                );
                            }
                        }

                        let mut incoming = TrackDetails {
                            ssrc,
                            kind: RTPCodecType::Video,
                            ..Default::default()
                        };
                        if only_media_section.media_name.media == RTPCodecType::Audio.to_string() {
                            incoming.kind = RTPCodecType::Audio;
                        }

                        let t = self
                            .add_transceiver_from_kind(
                                incoming.kind,
                                &[RTPTransceiverInit {
                                    direction: RTPTransceiverDirection::Sendrecv,
                                    send_encodings: vec![],
                                }],
                            )
                            .await?;

                        if let Some(receiver) = t.receiver().await {
                            PeerConnectionInternal::start_receiver(
                                &incoming,
                                receiver,
                                Arc::clone(&self.media_engine),
                                Arc::clone(&self.on_track_handler),
                            )
                            .await;
                        }
                        return Ok(());
                    }
                }

                let (mid_extension_id, audio_supported, video_supported) = self
                    .media_engine
                    .get_header_extension_id(RTPHeaderExtensionCapability {
                        uri: sdp::extmap::SDES_MID_URI.to_owned(),
                    })
                    .await;
                if !audio_supported && !video_supported {
                    return Err(Error::ErrPeerConnSimulcastMidRTPExtensionRequired.into());
                }

                let (sid_extension_id, audio_supported, video_supported) = self
                    .media_engine
                    .get_header_extension_id(RTPHeaderExtensionCapability {
                        uri: sdp::extmap::SDES_RTP_STREAM_ID_URI.to_owned(),
                    })
                    .await;
                if !audio_supported && !video_supported {
                    return Err(Error::ErrPeerConnSimulcastStreamIDRTPExtensionRequired.into());
                }

                let mut b = vec![0u8; RECEIVE_MTU];
                let (mut mid, mut rid) = (String::new(), String::new());
                for _ in 0..=SIMULCAST_PROBE_COUNT {
                    let n = rtp_stream.read(&mut b).await?;

                    let (maybe_mid, maybe_rid, payload_type) = handle_unknown_rtp_packet(
                        &b[..n],
                        mid_extension_id as u8,
                        sid_extension_id as u8,
                    )?;

                    if !maybe_mid.is_empty() {
                        mid = maybe_mid;
                    }
                    if !maybe_rid.is_empty() {
                        rid = maybe_rid;
                    }

                    if mid.is_empty() || rid.is_empty() {
                        continue;
                    }

                    let params = self
                        .media_engine
                        .get_rtp_parameters_by_payload_type(payload_type)
                        .await?;

                    {
                        let transceivers = self.rtp_transceivers.lock().await;
                        for t in &*transceivers {
                            if t.mid().await != mid || t.receiver().await.is_none() {
                                continue;
                            }

                            if let Some(receiver) = t.receiver().await {
                                let track = receiver
                                    .receive_for_rid(rid.as_str(), &params, ssrc)
                                    .await?;
                                PeerConnection::do_track(
                                    Arc::clone(&self.on_track_handler),
                                    Some(track),
                                    Some(receiver.clone()),
                                )
                                .await;
                            }
                            return Ok(());
                        }
                    }
                }
                return Err(Error::ErrPeerConnSimulcastIncomingSSRCFailed.into());
            }
        }

        Err(Error::ErrPeerConnRemoteDescriptionNil.into())
    }

    async fn start_receiver(
        incoming: &TrackDetails,
        receiver: Arc<RTPReceiver>,
        media_engine: Arc<MediaEngine>,
        on_track_handler: Arc<Mutex<Option<OnTrackHdlrFn>>>,
    ) {
        if receiver.start(incoming).await {
            tokio::spawn(async move {
                if let Some(track) = receiver.track().await {
                    if let Err(err) = track.determine_payload_type().await {
                        log::warn!(
                            "Could not determine PayloadType for SSRC {} with err {}",
                            track.ssrc(),
                            err
                        );
                        return;
                    }

                    let params = match media_engine
                        .get_rtp_parameters_by_payload_type(track.payload_type())
                        .await
                    {
                        Ok(params) => params,
                        Err(err) => {
                            log::warn!(
                                "no codec could be found for payloadType {} with err {}",
                                track.payload_type(),
                                err,
                            );
                            return;
                        }
                    };

                    track.set_kind(receiver.kind());
                    track.set_codec(params.codecs[0].clone()).await;
                    track.set_params(params).await;

                    PeerConnection::do_track(
                        on_track_handler,
                        receiver.track().await,
                        Some(Arc::clone(&receiver)),
                    )
                    .await;
                }
            });
        }
    }

    pub(super) async fn create_ice_transport(&self, api: &API) -> Arc<ICETransport> {
        let ice_transport = Arc::new(api.new_ice_transport(Arc::clone(&self.ice_gatherer)));

        let ice_connection_state = Arc::clone(&self.ice_connection_state);
        let peer_connection_state = Arc::clone(&self.peer_connection_state);
        let is_closed = Arc::clone(&self.is_closed);
        let dtls_transport = Arc::clone(&self.dtls_transport);
        let on_ice_connection_state_change_handler =
            Arc::clone(&self.on_ice_connection_state_change_handler);
        let on_peer_connection_state_change_handler =
            Arc::clone(&self.on_peer_connection_state_change_handler);

        ice_transport
            .on_connection_state_change(Box::new(move |state: ICETransportState| {
                let cs = match state {
                    ICETransportState::New => ICEConnectionState::New,
                    ICETransportState::Checking => ICEConnectionState::Checking,
                    ICETransportState::Connected => ICEConnectionState::Connected,
                    ICETransportState::Completed => ICEConnectionState::Completed,
                    ICETransportState::Failed => ICEConnectionState::Failed,
                    ICETransportState::Disconnected => ICEConnectionState::Disconnected,
                    ICETransportState::Closed => ICEConnectionState::Closed,
                    _ => {
                        log::warn!("on_connection_state_change: unhandled ICE state: {}", state);
                        return Box::pin(async {});
                    }
                };

                let ice_connection_state2 = Arc::clone(&ice_connection_state);
                let on_ice_connection_state_change_handler2 =
                    Arc::clone(&on_ice_connection_state_change_handler);
                let on_peer_connection_state_change_handler2 =
                    Arc::clone(&on_peer_connection_state_change_handler);
                let is_closed2 = Arc::clone(&is_closed);
                let dtls_transport_state = dtls_transport.state();
                let peer_connection_state2 = Arc::clone(&peer_connection_state);
                Box::pin(async move {
                    PeerConnection::do_ice_connection_state_change(
                        &on_ice_connection_state_change_handler2,
                        &ice_connection_state2,
                        cs,
                    )
                    .await;

                    PeerConnection::update_connection_state(
                        &on_peer_connection_state_change_handler2,
                        &is_closed2,
                        &peer_connection_state2,
                        cs,
                        dtls_transport_state,
                    )
                    .await;
                })
            }))
            .await;

        ice_transport
    }

    /// has_local_description_changed returns whether local media (rtp_transceivers) has changed
    /// caller of this method should hold `pc.mu` lock
    pub(super) async fn has_local_description_changed(&self, desc: &SessionDescription) -> bool {
        let rtp_transceivers = self.rtp_transceivers.lock().await;
        for t in &*rtp_transceivers {
            if let Some(m) = get_by_mid(t.mid().await.as_str(), desc) {
                if get_peer_direction(m) != t.direction() {
                    return true;
                }
            } else {
                return true;
            }
        }
        false
    }
}

#[async_trait]
impl RTCPWriter for PeerConnectionInternal {
    async fn write(
        &self,
        pkt: &(dyn rtcp::packet::Packet + Send + Sync),
        _a: &Attributes,
    ) -> Result<usize> {
        self.dtls_transport.write_rtcp(pkt).await
    }
}
