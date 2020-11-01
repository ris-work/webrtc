use std::fmt;
use url::Url;

use super::common_description::*;
use super::extmap::*;

// Constants for extmap key
const EXT_MAP_VALUE_TRANSPORT_CC_KEY: isize = 3;
const EXT_MAP_VALUE_TRANSPORT_CC_URI: &str =
    "http://www.ietf.org/id/draft-holmer-rmcat-transport-wide-cc-extensions-01";

// MediaDescription represents a media type.
// https://tools.ietf.org/html/rfc4566#section-5.14
#[derive(Debug, Default)]
pub struct MediaDescription {
    // m=<media> <port>/<number of ports> <proto> <fmt> ...
    // https://tools.ietf.org/html/rfc4566#section-5.14
    pub media_name: MediaName,

    // i=<session description>
    // https://tools.ietf.org/html/rfc4566#section-5.4
    pub media_title: Option<Information>,

    // c=<nettype> <addrtype> <connection-address>
    // https://tools.ietf.org/html/rfc4566#section-5.7
    pub connection_information: Option<ConnectionInformation>,

    // b=<bwtype>:<bandwidth>
    // https://tools.ietf.org/html/rfc4566#section-5.8
    pub bandwidth: Vec<Bandwidth>,

    // k=<method>
    // k=<method>:<encryption key>
    // https://tools.ietf.org/html/rfc4566#section-5.12
    pub encryption_key: Option<EncryptionKey>,

    // Attributes are the primary means for extending SDP.  Attributes may
    // be defined to be used as "session-level" attributes, "media-level"
    // attributes, or both.
    // https://tools.ietf.org/html/rfc4566#section-5.12
    pub attributes: Vec<Attribute>,
}

impl MediaDescription {
    // Attribute returns the value of an attribute and if it exists
    pub fn attribute(&self, key: &str) -> Option<&String> {
        for a in &self.attributes {
            if a.key == key {
                return a.value.as_ref();
            }
        }
        None
    }

    // New JSEPMediaDescription creates a new MediaName with
    // some settings that are required by the JSEP spec.
    // TODO: use codec_prefs
    pub fn new(codec_type: String, _codec_prefs: Vec<&str>) -> Self {
        // TODO: handle codec_prefs
        MediaDescription {
            media_name: MediaName {
                media: codec_type,
                port: RangedPort {
                    value: 9,
                    range: None,
                },
                protos: vec![
                    "UDP".to_string(),
                    "TLS".to_string(),
                    "RTP".to_string(),
                    "SAVPF".to_string(),
                ],
                formats: vec![],
            },
            media_title: None,
            connection_information: Some(ConnectionInformation {
                network_type: "IN".to_string(),
                address_type: "IP4".to_string(),
                address: Some(Address {
                    address: "0.0.0.0".to_string(),
                    ttl: None,
                    range: None,
                }),
            }),
            bandwidth: vec![],
            encryption_key: None,
            attributes: vec![],
        }
    }

    // WithPropertyAttribute adds a property attribute 'a=key' to the media description
    pub fn with_property_attribute(mut self, key: String) -> Self {
        self.attributes.push(Attribute::new(key, None));
        self
    }

    // WithValueAttribute adds a value attribute 'a=key:value' to the media description
    pub fn with_value_attribute(mut self, key: String, value: String) -> Self {
        self.attributes.push(Attribute::new(key, Some(value)));
        self
    }

    // WithICECredentials adds ICE credentials to the media description
    pub fn with_ice_credentials(self, username: String, password: String) -> Self {
        self.with_value_attribute("ice-ufrag".to_string(), username)
            .with_value_attribute("ice-pwd".to_string(), password)
    }

    // WithCodec adds codec information to the media description
    pub fn with_codec(
        mut self,
        payload_type: u8,
        name: String,
        clockrate: u32,
        channels: u16,
        fmtp: String,
    ) -> Self {
        self.media_name.formats.push(payload_type.to_string());
        let mut rtpmap = format!("{} {}/{}", payload_type, name, clockrate);
        if channels > 0 {
            rtpmap += format!("/{}", channels).as_str();
        }

        if &fmtp != "" {
            self.with_value_attribute("rtpmap".to_string(), rtpmap)
                .with_value_attribute("fmtp".to_string(), format!("{} {}", payload_type, fmtp))
        } else {
            self.with_value_attribute("rtpmap".to_string(), rtpmap)
        }
    }

    // WithMediaSource adds media source information to the media description
    pub fn with_media_source(
        self,
        ssrc: u32,
        cname: String,
        stream_label: String,
        label: String,
    ) -> Self {
        self.
            with_value_attribute("ssrc".to_string(), format!("{} cname:{}", ssrc, cname)). // Deprecated but not phased out?
            with_value_attribute("ssrc".to_string(), format!("{} msid:{} {}", ssrc, stream_label, label)).
            with_value_attribute("ssrc".to_string(), format!("{} mslabel:{}", ssrc, stream_label)). // Deprecated but not phased out?
            with_value_attribute("ssrc".to_string(), format!("{} label:{}", ssrc, label))
        // Deprecated but not phased out?
    }

    // WithCandidate adds an ICE candidate to the media description
    // Deprecated: use WithICECandidate instead
    pub fn with_candidate(self, value: String) -> Self {
        self.with_value_attribute("candidate".to_string(), value)
    }

    pub fn with_extmap(self, e: ExtMap) -> Self {
        self.with_property_attribute(e.marshal())
    }

    // WithTransportCCExtMap adds an extmap to the media description
    pub fn with_transport_cc_extmap(self) -> Self {
        let uri = match Url::parse(EXT_MAP_VALUE_TRANSPORT_CC_URI) {
            Ok(uri) => Some(uri),
            Err(_) => None,
        };

        let e = ExtMap {
            value: EXT_MAP_VALUE_TRANSPORT_CC_KEY,
            uri,
            ..Default::default()
        };

        self.with_extmap(e)
    }
}

// RangedPort supports special format for the media field "m=" port value. If
// it may be necessary to specify multiple transport ports, the protocol allows
// to write it as: <port>/<number of ports> where number of ports is a an
// offsetting range.
#[derive(Debug, Default)]
pub struct RangedPort {
    pub value: isize,
    pub range: Option<isize>,
}

impl fmt::Display for RangedPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(range) = self.range {
            write!(f, "{}/{}", self.value, range)
        } else {
            write!(f, "{}", self.value)
        }
    }
}

// MediaName describes the "m=" field storage structure.
#[derive(Debug, Default)]
pub struct MediaName {
    pub media: String,
    pub port: RangedPort,
    pub protos: Vec<String>,
    pub formats: Vec<String>,
}

impl fmt::Display for MediaName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = vec![
            self.media.clone(),
            self.port.to_string(),
            self.protos.join("/"),
            self.formats.join(" "),
        ];
        write!(f, "{}", s.join(" "))
    }
}
