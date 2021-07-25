use crate::media::dtls_transport::DTLSTransport;
use crate::media::ice_transport::ICETransport;
use crate::peer::ice::ice_gather::ice_gatherer::ICEGatherer;
use crate::peer::ice::ice_gather::ICEGatherOptions;

use dtls::crypto::Certificate;
use media_engine::*;
use setting_engine::*;

pub mod media_engine;
pub mod setting_engine;

use crate::data::data_channel::data_channel_parameters::DataChannelParameters;
use crate::data::data_channel::DataChannel;
use crate::data::sctp_transport::SCTPTransport;
use crate::error::Error;
use crate::media::interceptor::Interceptor;
use crate::media::rtp::rtp_codec::RTPCodecType;
use crate::media::rtp::rtp_receiver::RTPReceiver;

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

/// API bundles the global functions of the WebRTC and ORTC API.
/// Some of these functions are also exported globally using the
/// defaultAPI object. Note that the global version of the API
/// may be phased out in the future.
pub struct Api {
    setting_engine: Arc<SettingEngine>,
    media_engine: Arc<MediaEngine>,
    interceptor: Option<Arc<dyn Interceptor>>,
}

impl Api {
    /// new_ice_gatherer creates a new ice gatherer.
    /// This constructor is part of the ORTC API. It is not
    /// meant to be used together with the basic WebRTC API.
    pub fn new_ice_gatherer(&self, opts: ICEGatherOptions) -> Result<ICEGatherer> {
        let mut validated_servers = vec![];
        if !opts.ice_servers.is_empty() {
            for server in &opts.ice_servers {
                let url = server.urls()?;
                validated_servers.extend(url);
            }
        }

        Ok(ICEGatherer::new(
            validated_servers,
            opts.ice_gather_policy,
            Arc::clone(&self.setting_engine),
        ))
    }

    /// new_ice_transport creates a new ice transport.
    /// This constructor is part of the ORTC API. It is not
    /// meant to be used together with the basic WebRTC API.
    pub fn new_ice_transport(&self, gatherer: ICEGatherer) -> Result<ICETransport> {
        Ok(ICETransport::new(gatherer))
    }

    /// new_dtls_transport creates a new dtls_transport transport.
    /// This constructor is part of the ORTC API. It is not
    /// meant to be used together with the basic WebRTC API.
    pub fn new_dtls_transport(
        &self,
        ice_transport: ICETransport,
        certificates: Vec<Certificate>,
    ) -> Result<DTLSTransport> {
        /*TODO: if !certificates.is_empty() {
            now := time.Now()
            for _, x509Cert := range certificates {
                if !x509Cert.Expires().IsZero() && now.After(x509Cert.Expires()) {
                    return nil, &rtcerr.InvalidAccessError{Err: ErrCertificateExpired}
                }
                t.certificates = append(t.certificates, x509Cert)
            }
        } else {
            sk, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
            if err != nil {
                return nil, &rtcerr.UnknownError{Err: err}
            }
            certificate, err := GenerateCertificate(sk)
            if err != nil {
                return nil, err
            }
            t.certificates = []Certificate{*certificate}
        }*/

        Ok(DTLSTransport::new(
            ice_transport,
            certificates,
            Arc::clone(&self.setting_engine),
        ))
    }

    /// new_sctp_transport creates a new SCTPTransport.
    /// This constructor is part of the ORTC API. It is not
    /// meant to be used together with the basic WebRTC API.
    pub fn new_sctp_transport(&self, dtls_transport: Arc<DTLSTransport>) -> Result<SCTPTransport> {
        Ok(SCTPTransport::new(
            dtls_transport,
            Arc::clone(&self.setting_engine),
        ))
    }

    /// new_data_channel creates a new DataChannel.
    /// This constructor is part of the ORTC API. It is not
    /// meant to be used together with the basic WebRTC API.
    pub async fn new_data_channel(
        &self,
        sctp_transport: Arc<SCTPTransport>,
        params: DataChannelParameters,
    ) -> Result<DataChannel> {
        // https://w3c.github.io/webrtc-pc/#peer-to-peer-data-api (Step #5)
        if params.label.len() > 65535 {
            return Err(Error::ErrStringSizeLimit.into());
        }

        let d = DataChannel::new(params, Arc::clone(&self.setting_engine));
        d.open(sctp_transport).await?;

        Ok(d)
    }

    /// new_rtp_receiver constructs a new RTPReceiver
    pub fn new_rtp_receiver(
        &self,
        kind: RTPCodecType,
        transport: Arc<DTLSTransport>,
    ) -> RTPReceiver {
        let (closed_tx, closed_rx) = mpsc::channel(1);
        let (received_tx, received_rx) = mpsc::channel(1);

        RTPReceiver {
            kind,
            transport,

            tracks: vec![],

            closed_tx: Some(closed_tx),
            closed_rx,
            received_tx: Some(received_tx),
            received_rx,

            media_engine: Arc::clone(&self.media_engine),
            interceptor: self.interceptor.clone(),
        }
    }
}

#[derive(Default)]
pub struct ApiBuilder {
    setting_engine: Option<Arc<SettingEngine>>,
    media_engine: Option<Arc<MediaEngine>>,
    interceptor: Option<Arc<dyn Interceptor>>,
}

impl ApiBuilder {
    pub fn new() -> Self {
        ApiBuilder::default()
    }

    pub fn build(mut self) -> Api {
        Api {
            setting_engine: if let Some(setting_engine) = self.setting_engine.take() {
                setting_engine
            } else {
                Arc::new(SettingEngine::default())
            },
            media_engine: if let Some(media_engine) = self.media_engine.take() {
                media_engine
            } else {
                Arc::new(MediaEngine::default())
            },
            interceptor: self.interceptor.take(),
        }
    }

    /// WithSettingEngine allows providing a SettingEngine to the API.
    /// Settings should not be changed after passing the engine to an API.
    pub fn with_setting_engine(mut self, setting_engine: SettingEngine) -> Self {
        self.setting_engine = Some(Arc::new(setting_engine));
        self
    }

    /// WithMediaEngine allows providing a MediaEngine to the API.
    /// Settings can be changed after passing the engine to an API.
    pub fn with_media_engine(mut self, media_engine: MediaEngine) -> Self {
        self.media_engine = Some(Arc::new(media_engine));
        self
    }

    /// with_interceptor allows providing Interceptors to the API.
    /// Settings should not be changed after passing the registry to an API.
    pub fn with_interceptor(mut self, interceptor: Arc<dyn Interceptor>) -> Self {
        self.interceptor = Some(interceptor);
        self
    }
}
