use super::*;
use crate::compression_methods::*;
use crate::config::*;
use crate::conn::*;
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
use crate::handshake::*;
use crate::record_layer::record_layer_header::*;
use crate::record_layer::*;

use util::Error;

use std::sync::atomic::Ordering;

pub(crate) async fn flight1parse<C: FlightConn>(
    /*context.Context,*/
    _c: C,
    state: &mut State,
    cache: &HandshakeCache,
    cfg: &HandshakeConfig,
) -> Result<Flight, (Option<Alert>, Option<Error>)> {
    // HelloVerifyRequest can be skipped by the server,
    // so allow ServerHello during flight1 also
    let (seq, msgs) = match cache
        .full_pull_map(
            state.handshake_recv_sequence,
            &[
                HandshakeCachePullRule {
                    typ: HandshakeType::HelloVerifyRequest,
                    epoch: cfg.initial_epoch,
                    is_client: false,
                    optional: true,
                },
                HandshakeCachePullRule {
                    typ: HandshakeType::ServerHello,
                    epoch: cfg.initial_epoch,
                    is_client: false,
                    optional: true,
                },
            ],
        )
        .await
    {
        // No valid message received. Keep reading
        Ok((seq, msgs)) => (seq, msgs),
        Err(_) => return Err((None, None)),
    };

    if msgs.contains_key(&HandshakeType::ServerHello) {
        // Flight1 and flight2 were skipped.
        // Parse as flight3.
        // TODO: return flight3parse(c, state, cache, cfg);
    }

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
        Ok(Flight::Flight3)
    } else {
        Err((
            Some(Alert {
                alert_level: AlertLevel::Fatal,
                alert_description: AlertDescription::InternalError,
            }),
            None,
        ))
    }
}

pub(crate) async fn flight1generate<C: FlightConn>(
    _c: C,
    state: &mut State,
    _cache: &HandshakeCache,
    cfg: &HandshakeConfig,
) -> Result<Vec<Packet>, (Option<Alert>, Option<Error>)> {
    //TODO: figure out difference between golang's atom store and rust atom store
    let zero_epoch = 0;
    state.local_epoch.store(zero_epoch, Ordering::Relaxed);
    state.remote_epoch.store(zero_epoch, Ordering::Relaxed);

    state.named_curve = DEFAULT_NAMED_CURVE;
    state.cookie = vec![];
    state.local_random.populate();

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
