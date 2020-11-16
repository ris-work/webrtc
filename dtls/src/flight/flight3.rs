use super::*;
use crate::compression_methods::*;
use crate::config::*;
use crate::content::*;
use crate::curve::named_curve::*;
use crate::errors::*;
use crate::extension::extension_server_name::*;
use crate::extension::extension_supported_elliptic_curves::*;
use crate::extension::extension_supported_point_formats::*;
use crate::extension::extension_supported_signature_algorithms::*;
use crate::extension::extension_use_extended_master_secret::*;
use crate::extension::extension_use_srtp::*;
use crate::extension::*;
use crate::handshake::handshake_header::*;
use crate::handshake::handshake_message_client_hello::*;
use crate::handshake::handshake_message_server_key_exchange::*;
use crate::handshake::*;
use crate::record_layer::record_layer_header::*;
use crate::record_layer::*;

use crate::cipher_suite::cipher_suite_for_id;
use crate::prf::{prf_pre_master_secret, prf_psk_pre_master_secret};
use crate::{find_matching_cipher_suite, find_matching_srtp_profile};
use util::Error;

pub(crate) async fn flight3parse<C: FlightConn>(
    /*context.Context,*/
    _c: C,
    state: &mut State,
    cache: &HandshakeCache,
    cfg: &HandshakeConfig,
) -> Result<Flight, (Option<Alert>, Option<Error>)> {
    // Clients may receive multiple HelloVerifyRequest messages with different cookies.
    // Clients SHOULD handle this by sending a new ClientHello with a cookie in response
    // to the new HelloVerifyRequest. RFC 6347 Section 4.2.1
    if let Ok((seq, msgs)) = cache
        .full_pull_map(
            state.handshake_recv_sequence,
            &[HandshakeCachePullRule {
                typ: HandshakeType::HelloVerifyRequest,
                epoch: cfg.initial_epoch,
                is_client: false,
                optional: true,
            }],
        )
        .await
    {
        if let Some(message) = msgs.get(&HandshakeType::HelloVerifyRequest) {
            // DTLS 1.2 clients must not assume that the server will use the protocol version
            // specified in HelloVerifyRequest message. RFC 6347 Section 4.2.1
            let h = match message {
                HandshakeMessage::HelloVerifyRequest(h) => h,
                _ => {
                    return Err((
                        Some(Alert {
                            alert_level: AlertLevel::Fatal,
                            alert_description: AlertDescription::InternalError,
                        }),
                        None,
                    ))
                }
            };

            // DTLS 1.2 clients must not assume that the server will use the protocol version
            // specified in HelloVerifyRequest message. RFC 6347 Section 4.2.1
            if h.version != PROTOCOL_VERSION1_0 && h.version != PROTOCOL_VERSION1_2 {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::ProtocolVersion,
                    }),
                    Some(ERR_UNSUPPORTED_PROTOCOL_VERSION.clone()),
                ));
            }

            state.cookie.extend_from_slice(&h.cookie);
            state.handshake_recv_sequence = seq;
            return Ok(Flight::Flight3);
        }
    }

    let result = if cfg.local_psk_callback.is_none() {
        cache
            .full_pull_map(
                state.handshake_recv_sequence,
                &[
                    HandshakeCachePullRule {
                        typ: HandshakeType::ServerHello,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::ServerKeyExchange,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: true,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::ServerHelloDone,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                ],
            )
            .await
    } else {
        cache
            .full_pull_map(
                state.handshake_recv_sequence,
                &[
                    HandshakeCachePullRule {
                        typ: HandshakeType::ServerHello,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::Certificate,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: true,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::ServerKeyExchange,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::CertificateRequest,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: true,
                    },
                    HandshakeCachePullRule {
                        typ: HandshakeType::ServerHelloDone,
                        epoch: cfg.initial_epoch,
                        is_client: false,
                        optional: false,
                    },
                ],
            )
            .await
    };

    let (seq, msgs) = match result {
        Ok((seq, msgs)) => (seq, msgs),
        Err(_) => return Err((None, None)),
    };

    state.handshake_recv_sequence = seq;

    if let Some(message) = msgs.get(&HandshakeType::ServerHello) {
        let h = match message {
            HandshakeMessage::ServerHello(h) => h,
            _ => {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InternalError,
                    }),
                    None,
                ))
            }
        };

        if h.version != PROTOCOL_VERSION1_2 {
            return Err((
                Some(Alert {
                    alert_level: AlertLevel::Fatal,
                    alert_description: AlertDescription::ProtocolVersion,
                }),
                Some(ERR_UNSUPPORTED_PROTOCOL_VERSION.clone()),
            ));
        }

        for extension in &h.extensions {
            match extension {
                Extension::UseSRTP(e) => {
                    let profile = match find_matching_srtp_profile(
                        &e.protection_profiles,
                        &cfg.local_srtp_protection_profiles,
                    ) {
                        Ok(profile) => profile,
                        Err(_) => {
                            return Err((
                                Some(Alert {
                                    alert_level: AlertLevel::Fatal,
                                    alert_description: AlertDescription::IllegalParameter,
                                }),
                                Some(ERR_CLIENT_NO_MATCHING_SRTP_PROFILE.clone()),
                            ))
                        }
                    };
                    state.srtp_protection_profile = profile;
                }
                Extension::UseExtendedMasterSecret(_) => {
                    if cfg.extended_master_secret != ExtendedMasterSecretType::Disable {
                        state.extended_master_secret = true;
                    }
                }
                _ => {}
            };
        }

        if cfg.extended_master_secret == ExtendedMasterSecretType::Require
            && !state.extended_master_secret
        {
            return Err((
                Some(Alert {
                    alert_level: AlertLevel::Fatal,
                    alert_description: AlertDescription::InsufficientSecurity,
                }),
                Some(ERR_CLIENT_REQUIRED_BUT_NO_SERVER_EMS.clone()),
            ));
        }
        if !cfg.local_srtp_protection_profiles.is_empty()
            && state.srtp_protection_profile == SRTPProtectionProfile::Unsupported
        {
            return Err((
                Some(Alert {
                    alert_level: AlertLevel::Fatal,
                    alert_description: AlertDescription::InsufficientSecurity,
                }),
                Some(ERR_REQUESTED_BUT_NO_SRTP_EXTENSION.clone()),
            ));
        }
        if find_matching_cipher_suite(&[h.cipher_suite], &cfg.local_cipher_suites).is_err() {
            return Err((
                Some(Alert {
                    alert_level: AlertLevel::Fatal,
                    alert_description: AlertDescription::InsufficientSecurity,
                }),
                Some(ERR_CIPHER_SUITE_NO_INTERSECTION.clone()),
            ));
        }

        if let Ok(cipher_suite) = cipher_suite_for_id(h.cipher_suite) {
            state.cipher_suite = Some(cipher_suite);
        }
        state.remote_random = h.random.clone();
        //cfg.log.Tracef("[handshake] use cipher suite: %s", h.cipherSuite.String())
    }

    if let Some(message) = msgs.get(&HandshakeType::Certificate) {
        let h = match message {
            HandshakeMessage::Certificate(h) => h,
            _ => {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InternalError,
                    }),
                    None,
                ))
            }
        };
        state.peer_certificates = h.certificate.clone();
    }

    if let Some(message) = msgs.get(&HandshakeType::ServerKeyExchange) {
        let h = match message {
            HandshakeMessage::ServerKeyExchange(h) => h,
            _ => {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InternalError,
                    }),
                    None,
                ))
            }
        };

        if let Err((alert, err)) = handle_server_key_exchange(state, cfg, h) {
            return Err((alert, err));
        }
    }

    if let Some(message) = msgs.get(&HandshakeType::CertificateRequest) {
        match message {
            HandshakeMessage::CertificateRequest(_) => {}
            _ => {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InternalError,
                    }),
                    None,
                ))
            }
        };
        state.remote_requested_certificate = true;
    }

    Ok(Flight::Flight5)
}

pub(crate) async fn flight3generate<C: FlightConn>(
    _c: C,
    state: &mut State,
    _cache: &HandshakeCache,
    cfg: &HandshakeConfig,
) -> Result<Vec<Packet>, (Option<Alert>, Option<Error>)> {
    let mut extensions = vec![Extension::SupportedSignatureAlgorithms(
        ExtensionSupportedSignatureAlgorithms {
            signature_hash_algorithms: cfg.local_signature_schemes.clone(),
        },
    )];

    if cfg.local_psk_callback.is_none() {
        extensions.extend_from_slice(&[
            Extension::SupportedEllipticCurves(ExtensionSupportedEllipticCurves {
                elliptic_curves: vec![NamedCurve::X25519, NamedCurve::P256, NamedCurve::P384],
            }),
            Extension::SupportedPointFormats(ExtensionSupportedPointFormats {
                point_formats: vec![ELLIPTIC_CURVE_POINT_FORMAT_UNCOMPRESSED],
            }),
        ]);
    }

    if !cfg.local_srtp_protection_profiles.is_empty() {
        extensions.push(Extension::UseSRTP(ExtensionUseSRTP {
            protection_profiles: cfg.local_srtp_protection_profiles.clone(),
        }));
    }

    if cfg.extended_master_secret == ExtendedMasterSecretType::Request
        || cfg.extended_master_secret == ExtendedMasterSecretType::Require
    {
        extensions.push(Extension::UseExtendedMasterSecret(
            ExtensionUseExtendedMasterSecret { supported: true },
        ));
    }

    if !cfg.server_name.is_empty() {
        extensions.push(Extension::ServerName(ExtensionServerName {
            server_name: cfg.server_name.clone(),
        }));
    }

    Ok(vec![Packet {
        record: RecordLayer {
            record_layer_header: RecordLayerHeader {
                protocol_version: PROTOCOL_VERSION1_2,
                ..Default::default()
            },
            content: Content::Handshake(Handshake {
                handshake_header: HandshakeHeader::default(),
                handshake_message: HandshakeMessage::ClientHello(HandshakeMessageClientHello {
                    version: PROTOCOL_VERSION1_2,
                    random: state.local_random.clone(),
                    cookie: state.cookie.clone(),

                    cipher_suites: cfg.local_cipher_suites.clone(),
                    compression_methods: default_compression_methods(),
                    extensions,
                }),
            }),
        },
        should_encrypt: false,
        reset_local_sequence_number: false,
    }])
}

pub(crate) fn handle_server_key_exchange(
    state: &mut State,
    cfg: &HandshakeConfig,
    h: &HandshakeMessageServerKeyExchange,
) -> Result<(), (Option<Alert>, Option<Error>)> {
    if let Some(local_psk_callback) = &cfg.local_psk_callback {
        let psk = match local_psk_callback(&h.identity_hint) {
            Ok(psk) => psk,
            Err(err) => {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InternalError,
                    }),
                    Some(err),
                ))
            }
        };

        state.pre_master_secret = prf_psk_pre_master_secret(&psk);
    } else {
        let local_keypair = match h.named_curve.generate_keypair() {
            Ok(local_keypair) => local_keypair,
            Err(err) => {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InternalError,
                    }),
                    Some(err),
                ))
            }
        };

        state.pre_master_secret = match prf_pre_master_secret(
            &h.public_key,
            &local_keypair.private_key,
            local_keypair.curve,
        ) {
            Ok(pre_master_secret) => pre_master_secret,
            Err(err) => {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InternalError,
                    }),
                    Some(err),
                ))
            }
        };

        state.local_keypair = Some(local_keypair);
    }

    Ok(())
}
