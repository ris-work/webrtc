use super::*;
use crate::api::media_engine::{MIME_TYPE_OPUS, MIME_TYPE_VP8};
use crate::api::setting_engine::SettingEngine;
use crate::api::APIBuilder;
use crate::media::dtls_transport::dtls_role::DEFAULT_DTLS_ROLE_OFFER;
use crate::media::dtls_transport::RTCDtlsTransport;
use crate::media::rtp::rtp_sender::RTCRtpSender;
use crate::media::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use crate::media::track::track_local::TrackLocal;
use crate::peer::certificate::RTCCertificate;
use rcgen::KeyPair;
use sdp::common_description::Attribute;

#[test]
fn test_extract_fingerprint() -> Result<()> {
    //"Good Session Fingerprint"
    {
        let s = sdp::session_description::SessionDescription {
            attributes: vec![Attribute {
                key: "fingerprint".to_owned(),
                value: Some("foo bar".to_owned()),
            }],
            ..Default::default()
        };

        let (fingerprint, hash) = extract_fingerprint(&s)?;
        assert_eq!(fingerprint, "bar");
        assert_eq!(hash, "foo");
    }

    //"Good Media Fingerprint"
    {
        let s = sdp::session_description::SessionDescription {
            media_descriptions: vec![sdp::media_description::MediaDescription {
                attributes: vec![Attribute {
                    key: "fingerprint".to_owned(),
                    value: Some("foo bar".to_owned()),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let (fingerprint, hash) = extract_fingerprint(&s)?;
        assert_eq!(fingerprint, "bar");
        assert_eq!(hash, "foo");
    }

    //"No Fingerprint"
    {
        let s = sdp::session_description::SessionDescription::default();

        if let Err(err) = extract_fingerprint(&s) {
            assert!(Error::ErrSessionDescriptionNoFingerprint.equal(&err));
        } else {
            assert!(false);
        }
    }

    //"Invalid Fingerprint"
    {
        let s = sdp::session_description::SessionDescription {
            attributes: vec![Attribute {
                key: "fingerprint".to_owned(),
                value: Some("foo".to_owned()),
            }],
            ..Default::default()
        };

        if let Err(err) = extract_fingerprint(&s) {
            assert!(Error::ErrSessionDescriptionInvalidFingerprint.equal(&err));
        } else {
            assert!(false);
        }
    }

    //"Conflicting Fingerprint"
    {
        let s = sdp::session_description::SessionDescription {
            attributes: vec![Attribute {
                key: "fingerprint".to_owned(),
                value: Some("foo".to_owned()),
            }],
            media_descriptions: vec![sdp::media_description::MediaDescription {
                attributes: vec![Attribute {
                    key: "fingerprint".to_owned(),
                    value: Some("foo bar".to_owned()),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        if let Err(err) = extract_fingerprint(&s) {
            assert!(Error::ErrSessionDescriptionConflictingFingerprints.equal(&err));
        } else {
            assert!(false);
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_extract_ice_details() -> Result<()> {
    const DEFAULT_UFRAG: &str = "DEFAULT_PWD";
    const DEFAULT_PWD: &str = "DEFAULT_UFRAG";

    //"Missing ice-pwd"
    {
        let s = sdp::session_description::SessionDescription {
            media_descriptions: vec![sdp::media_description::MediaDescription {
                attributes: vec![Attribute {
                    key: "ice-ufrag".to_owned(),
                    value: Some(DEFAULT_UFRAG.to_owned()),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        if let Err(err) = extract_ice_details(&s).await {
            assert!(Error::ErrSessionDescriptionMissingIcePwd.equal(&err));
        } else {
            assert!(false);
        }
    }

    //"Missing ice-ufrag"
    {
        let s = sdp::session_description::SessionDescription {
            media_descriptions: vec![sdp::media_description::MediaDescription {
                attributes: vec![Attribute {
                    key: "ice-pwd".to_owned(),
                    value: Some(DEFAULT_PWD.to_owned()),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        if let Err(err) = extract_ice_details(&s).await {
            assert!(Error::ErrSessionDescriptionMissingIceUfrag.equal(&err));
        } else {
            assert!(false);
        }
    }

    //"ice details at session level"
    {
        let s = sdp::session_description::SessionDescription {
            attributes: vec![
                Attribute {
                    key: "ice-ufrag".to_owned(),
                    value: Some(DEFAULT_UFRAG.to_owned()),
                },
                Attribute {
                    key: "ice-pwd".to_owned(),
                    value: Some(DEFAULT_PWD.to_owned()),
                },
            ],
            media_descriptions: vec![],
            ..Default::default()
        };

        let (ufrag, pwd, _) = extract_ice_details(&s).await?;
        assert_eq!(ufrag, DEFAULT_UFRAG);
        assert_eq!(pwd, DEFAULT_PWD);
    }

    //"ice details at media level"
    {
        let s = sdp::session_description::SessionDescription {
            media_descriptions: vec![sdp::media_description::MediaDescription {
                attributes: vec![
                    Attribute {
                        key: "ice-ufrag".to_owned(),
                        value: Some(DEFAULT_UFRAG.to_owned()),
                    },
                    Attribute {
                        key: "ice-pwd".to_owned(),
                        value: Some(DEFAULT_PWD.to_owned()),
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let (ufrag, pwd, _) = extract_ice_details(&s).await?;
        assert_eq!(ufrag, DEFAULT_UFRAG);
        assert_eq!(pwd, DEFAULT_PWD);
    }

    //"Conflict ufrag"
    {
        let s = sdp::session_description::SessionDescription {
            attributes: vec![Attribute {
                key: "ice-ufrag".to_owned(),
                value: Some("invalidUfrag".to_owned()),
            }],
            media_descriptions: vec![sdp::media_description::MediaDescription {
                attributes: vec![
                    Attribute {
                        key: "ice-ufrag".to_owned(),
                        value: Some(DEFAULT_UFRAG.to_owned()),
                    },
                    Attribute {
                        key: "ice-pwd".to_owned(),
                        value: Some(DEFAULT_PWD.to_owned()),
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        if let Err(err) = extract_ice_details(&s).await {
            assert!(Error::ErrSessionDescriptionConflictingIceUfrag.equal(&err));
        } else {
            assert!(false);
        }
    }

    //"Conflict pwd"
    {
        let s = sdp::session_description::SessionDescription {
            attributes: vec![Attribute {
                key: "ice-pwd".to_owned(),
                value: Some("invalidPwd".to_owned()),
            }],
            media_descriptions: vec![sdp::media_description::MediaDescription {
                attributes: vec![
                    Attribute {
                        key: "ice-ufrag".to_owned(),
                        value: Some(DEFAULT_UFRAG.to_owned()),
                    },
                    Attribute {
                        key: "ice-pwd".to_owned(),
                        value: Some(DEFAULT_PWD.to_owned()),
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        if let Err(err) = extract_ice_details(&s).await {
            assert!(Error::ErrSessionDescriptionConflictingIcePwd.equal(&err));
        } else {
            assert!(false);
        }
    }

    Ok(())
}

#[test]
fn test_track_details_from_sdp() -> Result<()> {
    //"Tracks unknown, audio and video with RTX"
    {
        let s = sdp::session_description::SessionDescription {
            media_descriptions: vec![
                sdp::media_description::MediaDescription {
                    media_name: MediaName {
                        media: "foobar".to_owned(),
                        ..Default::default()
                    },
                    attributes: vec![
                        Attribute {
                            key: "mid".to_owned(),
                            value: Some("0".to_owned()),
                        },
                        Attribute {
                            key: "sendrecv".to_owned(),
                            value: None,
                        },
                        Attribute {
                            key: "ssrc".to_owned(),
                            value: Some("1000 msid:unknown_trk_label unknown_trk_guid".to_owned()),
                        },
                    ],
                    ..Default::default()
                },
                sdp::media_description::MediaDescription {
                    media_name: MediaName {
                        media: "audio".to_owned(),
                        ..Default::default()
                    },
                    attributes: vec![
                        Attribute {
                            key: "mid".to_owned(),
                            value: Some("1".to_owned()),
                        },
                        Attribute {
                            key: "sendrecv".to_owned(),
                            value: None,
                        },
                        Attribute {
                            key: "ssrc".to_owned(),
                            value: Some("2000 msid:audio_trk_label audio_trk_guid".to_owned()),
                        },
                    ],
                    ..Default::default()
                },
                sdp::media_description::MediaDescription {
                    media_name: MediaName {
                        media: "video".to_owned(),
                        ..Default::default()
                    },
                    attributes: vec![
                        Attribute {
                            key: "mid".to_owned(),
                            value: Some("2".to_owned()),
                        },
                        Attribute {
                            key: "sendrecv".to_owned(),
                            value: None,
                        },
                        Attribute {
                            key: "ssrc-group".to_owned(),
                            value: Some("FID 3000 4000".to_owned()),
                        },
                        Attribute {
                            key: "ssrc".to_owned(),
                            value: Some("3000 msid:video_trk_label video_trk_guid".to_owned()),
                        },
                        Attribute {
                            key: "ssrc".to_owned(),
                            value: Some("4000 msid:rtx_trk_label rtx_trck_guid".to_owned()),
                        },
                    ],
                    ..Default::default()
                },
                sdp::media_description::MediaDescription {
                    media_name: MediaName {
                        media: "video".to_owned(),
                        ..Default::default()
                    },
                    attributes: vec![
                        Attribute {
                            key: "mid".to_owned(),
                            value: Some("3".to_owned()),
                        },
                        Attribute {
                            key: "sendonly".to_owned(),
                            value: None,
                        },
                        Attribute {
                            key: "msid".to_owned(),
                            value: Some("video_stream_id video_trk_id".to_owned()),
                        },
                        Attribute {
                            key: "ssrc".to_owned(),
                            value: Some("5000".to_owned()),
                        },
                    ],
                    ..Default::default()
                },
                sdp::media_description::MediaDescription {
                    media_name: MediaName {
                        media: "video".to_owned(),
                        ..Default::default()
                    },
                    attributes: vec![
                        Attribute {
                            key: "sendonly".to_owned(),
                            value: None,
                        },
                        Attribute {
                            key: "rid".to_owned(),
                            value: Some("f send pt=97;max-width=1280;max-height=720".to_owned()),
                        },
                    ],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let tracks = track_details_from_sdp(&s);
        assert_eq!(3, tracks.len());
        if track_details_for_ssrc(&tracks, 1000).is_some() {
            assert!(
                false,
                "got the unknown track ssrc:1000 which should have been skipped"
            );
        }
        if let Some(track) = track_details_for_ssrc(&tracks, 2000) {
            assert_eq!(RTPCodecType::Audio, track.kind);
            assert_eq!(2000, track.ssrc);
            assert_eq!("audio_trk_label", track.stream_id);
        } else {
            assert!(false, "missing audio track with ssrc:2000");
        }
        if let Some(track) = track_details_for_ssrc(&tracks, 3000) {
            assert_eq!(RTPCodecType::Video, track.kind);
            assert_eq!(3000, track.ssrc);
            assert_eq!("video_trk_label", track.stream_id);
        } else {
            assert!(false, "missing video track with ssrc:3000");
        }
        if track_details_for_ssrc(&tracks, 4000).is_some() {
            assert!(
                false,
                "got the rtx track ssrc:3000 which should have been skipped"
            );
        }
        if let Some(track) = track_details_for_ssrc(&tracks, 5000) {
            assert_eq!(RTPCodecType::Video, track.kind);
            assert_eq!(5000, track.ssrc);
            assert_eq!("video_trk_id", track.id);
            assert_eq!("video_stream_id", track.stream_id);
        } else {
            assert!(false, "missing video track with ssrc:5000");
        }
    }

    //"inactive and recvonly tracks ignored"
    {
        let s = sdp::session_description::SessionDescription {
            media_descriptions: vec![
                sdp::media_description::MediaDescription {
                    media_name: MediaName {
                        media: "video".to_owned(),
                        ..Default::default()
                    },
                    attributes: vec![
                        Attribute {
                            key: "inactive".to_owned(),
                            value: None,
                        },
                        Attribute {
                            key: "ssrc".to_owned(),
                            value: Some("6000".to_owned()),
                        },
                    ],
                    ..Default::default()
                },
                sdp::media_description::MediaDescription {
                    media_name: MediaName {
                        media: "video".to_owned(),
                        ..Default::default()
                    },
                    attributes: vec![
                        Attribute {
                            key: "recvonly".to_owned(),
                            value: None,
                        },
                        Attribute {
                            key: "ssrc".to_owned(),
                            value: Some("7000".to_owned()),
                        },
                    ],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(0, track_details_from_sdp(&s).len());
    }

    Ok(())
}

#[test]
fn test_have_application_media_section() -> Result<()> {
    //"Audio only"
    {
        let s = sdp::session_description::SessionDescription {
            media_descriptions: vec![sdp::media_description::MediaDescription {
                media_name: MediaName {
                    media: "audio".to_owned(),
                    ..Default::default()
                },
                attributes: vec![
                    Attribute {
                        key: "sendrecv".to_owned(),
                        value: None,
                    },
                    Attribute {
                        key: "ssrc".to_owned(),
                        value: Some("2000".to_owned()),
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(!have_application_media_section(&s));
    }

    //"Application"
    {
        let s = sdp::session_description::SessionDescription {
            media_descriptions: vec![sdp::media_description::MediaDescription {
                media_name: MediaName {
                    media: MEDIA_SECTION_APPLICATION.to_owned(),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };

        assert!(have_application_media_section(&s));
    }

    Ok(())
}

async fn fingerprint_test(
    certificate: &RTCCertificate,
    engine: &Arc<MediaEngine>,
    media: &[MediaSection],
    sdpmedia_description_fingerprints: bool,
    expected_fingerprint_count: usize,
) -> Result<()> {
    let s = sdp::session_description::SessionDescription::default();

    let dtls_fingerprints = certificate.get_fingerprints()?;

    let params = PopulateSdpParams {
        is_plan_b: false,
        media_description_fingerprint: sdpmedia_description_fingerprints,
        is_icelite: false,
        connection_role: ConnectionRole::Active,
        ice_gathering_state: RTCIceGatheringState::New,
    };

    let s = populate_sdp(
        s,
        &dtls_fingerprints,
        engine,
        &[],
        &RTCIceParameters::default(),
        media,
        params,
    )
    .await?;

    let sdparray = s.marshal();

    let v: Vec<&str> = sdparray.matches("sha-256").collect();
    assert_eq!(v.len(), expected_fingerprint_count);

    Ok(())
}

#[tokio::test]
async fn test_media_description_fingerprints() -> Result<()> {
    let mut m = MediaEngine::default();
    m.register_default_codecs()?;
    let api = APIBuilder::new().with_media_engine(m).build();

    let kp = KeyPair::generate(&rcgen::PKCS_ECDSA_P256_SHA256)?;
    let certificate = RTCCertificate::from_key_pair(kp)?;

    let media = vec![
        MediaSection {
            id: "video".to_owned(),
            transceivers: vec![
                RTCRtpTransceiver::new(
                    None,
                    None,
                    RTCRtpTransceiverDirection::Inactive,
                    RTPCodecType::Video,
                    api.media_engine
                        .get_codecs_by_kind(RTPCodecType::Video)
                        .await,
                    Arc::clone(&api.media_engine),
                )
                .await,
            ],
            ..Default::default()
        },
        MediaSection {
            id: "audio".to_owned(),
            transceivers: vec![
                RTCRtpTransceiver::new(
                    None,
                    None,
                    RTCRtpTransceiverDirection::Inactive,
                    RTPCodecType::Audio,
                    api.media_engine
                        .get_codecs_by_kind(RTPCodecType::Audio)
                        .await,
                    Arc::clone(&api.media_engine),
                )
                .await,
            ],
            ..Default::default()
        },
        MediaSection {
            id: "application".to_owned(),
            data: true,
            ..Default::default()
        },
    ];

    for i in 0..2 {
        let track: Arc<dyn TrackLocal + Send + Sync> = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: "video/vp8".to_owned(),
                ..Default::default()
            },
            "video".to_owned(),
            "webrtc-rs".to_owned(),
        ));
        media[i].transceivers[0]
            .set_sender(Some(Arc::new(
                RTCRtpSender::new(
                    track,
                    Arc::new(RTCDtlsTransport::default()),
                    Arc::clone(&api.media_engine),
                    Arc::clone(&api.interceptor),
                )
                .await,
            )))
            .await;
        media[i].transceivers[0].set_direction(RTCRtpTransceiverDirection::Sendonly);
    }

    //"Per-Media Description Fingerprints",
    fingerprint_test(&certificate, &api.media_engine, &media, true, 3).await?;

    //"Per-Session Description Fingerprints",
    fingerprint_test(&certificate, &api.media_engine, &media, false, 1).await?;

    Ok(())
}

#[tokio::test]
async fn test_populate_sdp() -> Result<()> {
    //"Rid"
    {
        let se = SettingEngine::default();
        let mut me = MediaEngine::default();
        me.register_default_codecs()?;
        let me = Arc::new(me);

        let tr = RTCRtpTransceiver::new(
            None,
            None,
            RTCRtpTransceiverDirection::Recvonly,
            RTPCodecType::Video,
            me.video_codecs.clone(),
            Arc::clone(&me),
        )
        .await;

        let mut rid_map = HashMap::new();
        rid_map.insert("ridkey".to_owned(), "some".to_owned());
        let media_sections = vec![MediaSection {
            id: "video".to_owned(),
            transceivers: vec![tr],
            data: false,
            rid_map,
        }];

        let d = sdp::session_description::SessionDescription::default();

        let params = PopulateSdpParams {
            is_plan_b: false,
            media_description_fingerprint: se.sdp_media_level_fingerprints,
            is_icelite: se.candidates.ice_lite,
            connection_role: DEFAULT_DTLS_ROLE_OFFER.to_connection_role(),
            ice_gathering_state: RTCIceGatheringState::Complete,
        };
        let offer_sdp = populate_sdp(
            d,
            &[],
            &me,
            &[],
            &RTCIceParameters::default(),
            &media_sections,
            params,
        )
        .await?;

        // Test contains rid map keys
        let mut found = false;
        for desc in &offer_sdp.media_descriptions {
            if desc.media_name.media != "video" {
                continue;
            }
            for a in &desc.attributes {
                if a.key == "rid" {
                    if let Some(value) = &a.value {
                        if value.contains("ridkey") {
                            found = true;
                            break;
                        }
                    }
                }
            }
        }
        assert_eq!(true, found, "Rid key should be present");
    }

    //"SetCodecPreferences"
    {
        let se = SettingEngine::default();
        let mut me = MediaEngine::default();
        me.register_default_codecs()?;
        me.push_codecs(me.video_codecs.clone(), RTPCodecType::Video)
            .await;
        me.push_codecs(me.audio_codecs.clone(), RTPCodecType::Audio)
            .await;
        let me = Arc::new(me);

        let tr = RTCRtpTransceiver::new(
            None,
            None,
            RTCRtpTransceiverDirection::Recvonly,
            RTPCodecType::Video,
            me.video_codecs.clone(),
            Arc::clone(&me),
        )
        .await;
        tr.set_codec_preferences(vec![RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_VP8.to_owned(),
                clock_rate: 90000,
                channels: 0,
                sdp_fmtp_line: "".to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: 96,
            ..Default::default()
        }])
        .await?;

        let media_sections = vec![MediaSection {
            id: "video".to_owned(),
            transceivers: vec![tr],
            data: false,
            rid_map: HashMap::new(),
        }];

        let d = sdp::session_description::SessionDescription::default();

        let params = PopulateSdpParams {
            is_plan_b: false,
            media_description_fingerprint: se.sdp_media_level_fingerprints,
            is_icelite: se.candidates.ice_lite,
            connection_role: DEFAULT_DTLS_ROLE_OFFER.to_connection_role(),
            ice_gathering_state: RTCIceGatheringState::Complete,
        };
        let offer_sdp = populate_sdp(
            d,
            &[],
            &me,
            &[],
            &RTCIceParameters::default(),
            &media_sections,
            params,
        )
        .await?;

        // Test codecs
        let mut found_vp8 = false;
        for desc in &offer_sdp.media_descriptions {
            if desc.media_name.media != "video" {
                continue;
            }
            for a in &desc.attributes {
                if a.key.contains("rtpmap") {
                    if let Some(value) = &a.value {
                        if value == "98 VP9/90000" {
                            assert!(false, "vp9 should not be present in sdp");
                        } else if value == "96 VP8/90000" {
                            found_vp8 = true;
                        }
                    }
                }
            }
        }
        assert_eq!(true, found_vp8, "vp8 should be present in sdp");
    }

    Ok(())
}

#[test]
fn test_get_rids() {
    let m = vec![sdp::media_description::MediaDescription {
        media_name: MediaName {
            media: "video".to_owned(),
            ..Default::default()
        },
        attributes: vec![
            Attribute {
                key: "sendonly".to_owned(),
                value: None,
            },
            Attribute {
                key: "rid".to_owned(),
                value: Some("f send pt=97;max-width=1280;max-height=720".to_owned()),
            },
        ],
        ..Default::default()
    }];

    let rids = get_rids(&m[0]);

    assert!(!rids.is_empty(), "Rid mapping should be present");

    assert!(rids.get("f").is_some(), "rid values should contain 'f'");
}

#[test]
fn test_codecs_from_media_description() -> Result<()> {
    //"Codec Only"
    {
        let codecs = codecs_from_media_description(&sdp::media_description::MediaDescription {
            media_name: MediaName {
                media: "audio".to_owned(),
                formats: vec!["111".to_owned()],
                ..Default::default()
            },
            attributes: vec![Attribute {
                key: "rtpmap".to_owned(),
                value: Some("111 opus/48000/2".to_owned()),
            }],
            ..Default::default()
        })?;

        assert_eq!(
            codecs,
            vec![RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_OPUS.to_owned(),
                    clock_rate: 48000,
                    channels: 2,
                    sdp_fmtp_line: "".to_owned(),
                    rtcp_feedback: vec![],
                },
                payload_type: 111,
                ..Default::default()
            }],
        );
    }

    //"Codec with fmtp/rtcp-fb"
    {
        let codecs = codecs_from_media_description(&sdp::media_description::MediaDescription {
            media_name: MediaName {
                media: "audio".to_owned(),
                formats: vec!["111".to_owned()],
                ..Default::default()
            },
            attributes: vec![
                Attribute {
                    key: "rtpmap".to_owned(),
                    value: Some("111 opus/48000/2".to_owned()),
                },
                Attribute {
                    key: "fmtp".to_owned(),
                    value: Some("111 minptime=10;useinbandfec=1".to_owned()),
                },
                Attribute {
                    key: "rtcp-fb".to_owned(),
                    value: Some("111 goog-remb".to_owned()),
                },
                Attribute {
                    key: "rtcp-fb".to_owned(),
                    value: Some("111 ccm fir".to_owned()),
                },
            ],
            ..Default::default()
        })?;

        assert_eq!(
            codecs,
            vec![RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_OPUS.to_owned(),
                    clock_rate: 48000,
                    channels: 2,
                    sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
                    rtcp_feedback: vec![
                        RTCPFeedback {
                            typ: "goog-remb".to_owned(),
                            parameter: "".to_owned()
                        },
                        RTCPFeedback {
                            typ: "ccm".to_owned(),
                            parameter: "fir".to_owned()
                        }
                    ]
                },
                payload_type: 111,
                ..Default::default()
            }],
        );
    }

    Ok(())
}

#[test]
fn test_rtp_extensions_from_media_description() -> Result<()> {
    let extensions =
        rtp_extensions_from_media_description(&sdp::media_description::MediaDescription {
            media_name: MediaName {
                media: "audio".to_owned(),
                formats: vec!["111".to_owned()],
                ..Default::default()
            },
            attributes: vec![
                Attribute {
                    key: "extmap".to_owned(),
                    value: Some("1 ".to_owned() + sdp::extmap::ABS_SEND_TIME_URI),
                },
                Attribute {
                    key: "extmap".to_owned(),
                    value: Some("3 ".to_owned() + sdp::extmap::SDES_MID_URI),
                },
            ],
            ..Default::default()
        })?;

    assert_eq!(extensions[sdp::extmap::ABS_SEND_TIME_URI], 1);
    assert_eq!(extensions[sdp::extmap::SDES_MID_URI], 3);

    Ok(())
}
