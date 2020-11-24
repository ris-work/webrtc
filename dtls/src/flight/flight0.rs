use super::flight2::*;
use super::*;
use crate::config::*;
use crate::conn::*;
use crate::errors::*;
use crate::extension::*;
use crate::handshake::*;
use crate::record_layer::record_layer_header::*;
use crate::*;

use util::Error;

use rand::Rng;

use std::sync::atomic::Ordering;

use async_trait::async_trait;

pub(crate) struct Flight0;

#[async_trait]
impl Flight for Flight0 {
    fn to_string(&self) -> String {
        "Flight0".to_owned()
    }

    async fn parse(
        &self,
        _c: &mut Conn,
        state: &mut State,
        cache: &HandshakeCache,
        cfg: &HandshakeConfig,
    ) -> Result<Box<dyn Flight>, (Option<Alert>, Option<Error>)> {
        let (seq, msgs) = match cache
            .full_pull_map(
                0,
                &[HandshakeCachePullRule {
                    typ: HandshakeType::ClientHello,
                    epoch: cfg.initial_epoch,
                    is_client: true,
                    optional: false,
                }],
            )
            .await
        {
            Ok((seq, msgs)) => (seq, msgs),
            Err(_) => return Err((None, None)),
        };

        state.handshake_recv_sequence = seq;

        if let Some(message) = msgs.get(&HandshakeType::ClientHello) {
            // Validate type
            let client_hello = match message {
                HandshakeMessage::ClientHello(client_hello) => client_hello,
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

            if client_hello.version != PROTOCOL_VERSION1_2 {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::ProtocolVersion,
                    }),
                    Some(ERR_UNSUPPORTED_PROTOCOL_VERSION.clone()),
                ));
            }

            state.remote_random = client_hello.random.clone();

            if let Ok(id) =
                find_matching_cipher_suite(&client_hello.cipher_suites, &cfg.local_cipher_suites)
            {
                if let Ok(cipher_suite) = cipher_suite_for_id(id) {
                    state.cipher_suite = Some(cipher_suite);
                }
            } else {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InsufficientSecurity,
                    }),
                    Some(ERR_CIPHER_SUITE_NO_INTERSECTION.clone()),
                ));
            }

            for extension in &client_hello.extensions {
                match extension {
                    Extension::SupportedEllipticCurves(e) => {
                        if e.elliptic_curves.is_empty() {
                            return Err((
                                Some(Alert {
                                    alert_level: AlertLevel::Fatal,
                                    alert_description: AlertDescription::InsufficientSecurity,
                                }),
                                Some(ERR_NO_SUPPORTED_ELLIPTIC_CURVES.clone()),
                            ));
                        }
                        state.named_curve = e.elliptic_curves[0];
                    }
                    Extension::UseSRTP(e) => {
                        if let Ok(profile) = find_matching_srtp_profile(
                            &e.protection_profiles,
                            &cfg.local_srtp_protection_profiles,
                        ) {
                            state.srtp_protection_profile = profile;
                        } else {
                            return Err((
                                Some(Alert {
                                    alert_level: AlertLevel::Fatal,
                                    alert_description: AlertDescription::InsufficientSecurity,
                                }),
                                Some(ERR_SERVER_NO_MATCHING_SRTP_PROFILE.clone()),
                            ));
                        }
                    }
                    Extension::UseExtendedMasterSecret(_) => {
                        if cfg.extended_master_secret != ExtendedMasterSecretType::Disable {
                            state.extended_master_secret = true;
                        }
                    }
                    Extension::ServerName(e) => {
                        state.server_name = e.server_name.clone(); // remote server name
                    }
                    _ => {}
                }
            }

            if cfg.extended_master_secret == ExtendedMasterSecretType::Require
                && !state.extended_master_secret
            {
                return Err((
                    Some(Alert {
                        alert_level: AlertLevel::Fatal,
                        alert_description: AlertDescription::InsufficientSecurity,
                    }),
                    Some(ERR_SERVER_REQUIRED_BUT_NO_CLIENT_EMS.clone()),
                ));
            }

            if state.local_keypair.is_none() {
                state.local_keypair = match state.named_curve.generate_keypair() {
                    Ok(local_keypar) => Some(local_keypar),
                    Err(err) => {
                        return Err((
                            Some(Alert {
                                alert_level: AlertLevel::Fatal,
                                alert_description: AlertDescription::IllegalParameter,
                            }),
                            Some(err),
                        ))
                    }
                };
            }

            Ok(Box::new(Flight2 {}))
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

    async fn generate(
        &self,
        state: &mut State,
        _cache: &HandshakeCache,
        _cfg: &HandshakeConfig,
    ) -> Result<Vec<Packet>, (Option<Alert>, Option<Error>)> {
        // Initialize
        rand::thread_rng().fill(state.cookie.as_mut_slice());

        //TODO: figure out difference between golang's atom store and rust atom store
        let zero_epoch = 0;
        state.local_epoch.store(zero_epoch, Ordering::Relaxed);
        state.remote_epoch.store(zero_epoch, Ordering::Relaxed);

        state.named_curve = DEFAULT_NAMED_CURVE;
        state.local_random.populate();

        Ok(vec![])
    }
}
