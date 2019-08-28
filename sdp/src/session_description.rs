use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fmt, io};

use url::Url;
use util::Error;

use super::common_description::*;
use super::media_description::*;
use super::util::*;

#[cfg(test)]
mod session_description_test;

// Constants for SDP attributes used in JSEP
const ATTR_KEY_IDENTITY: &'static str = "identity";
const ATTR_KEY_GROUP: &'static str = "group";
const ATTR_KEY_SSRC: &'static str = "ssrc";
const ATTR_KEY_SSRCGROUP: &'static str = "ssrc-group";
const ATTR_KEY_MSID_SEMANTIC: &'static str = "msid-semantic";
const ATTR_KEY_CONNECTION_SETUP: &'static str = "setup";
const ATTR_KEY_MID: &'static str = "mid";
const ATTR_KEY_ICELITE: &'static str = "ice-lite";
const ATTR_KEY_RTCPMUX: &'static str = "rtcp-mux";
const ATTR_KEY_RTCPRSIZE: &'static str = "rtcp-rsize";

// Constants for semantic tokens used in JSEP
const SEMANTIC_TOKEN_LIP_SYNCHRONIZATION: &'static str = "LS";
const SEMANTIC_TOKEN_FLOW_IDENTIFICATION: &'static str = "FID";
const SEMANTIC_TOKEN_FORWARD_ERROR_CORRECTION: &'static str = "FEC";
const SEMANTIC_TOKEN_WEB_RTCMEDIA_STREAMS: &'static str = "WMS";

// Version describes the value provided by the "v=" field which gives
// the version of the Session Description Protocol.
pub type Version = isize;

// Origin defines the structure for the "o=" field which provides the
// originator of the session plus a session identifier and version number.
#[derive(Debug, Default)]
pub struct Origin {
    username: String,
    session_id: u64,
    session_version: u64,
    network_type: String,
    address_type: String,
    unicast_address: String,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} {} {} {} {} {}",
            self.username,
            self.session_id,
            self.session_version,
            self.network_type,
            self.address_type,
            self.unicast_address,
        )
    }
}

impl Origin {
    pub fn new() -> Self {
        Origin {
            username: "".to_owned(),
            session_id: 0,
            session_version: 0,
            network_type: "".to_owned(),
            address_type: "".to_owned(),
            unicast_address: "".to_owned(),
        }
    }
}

// SessionName describes a structured representations for the "s=" field
// and is the textual session name.
pub type SessionName = String;

// EmailAddress describes a structured representations for the "e=" line
// which specifies email contact information for the person responsible for
// the conference.
pub type EmailAddress = String;

// PhoneNumber describes a structured representations for the "p=" line
// specify phone contact information for the person responsible for the
// conference.
pub type PhoneNumber = String;

// TimeZone defines the structured object for "z=" line which describes
// repeated sessions scheduling.
#[derive(Debug, Default)]
pub struct TimeZone {
    adjustment_time: u64,
    offset: i64,
}

impl fmt::Display for TimeZone {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.adjustment_time, self.offset)
    }
}

// TimeDescription describes "t=", "r=" fields of the session description
// which are used to specify the start and stop times for a session as well as
// repeat intervals and durations for the scheduled session.
#[derive(Debug, Default)]
pub struct TimeDescription {
    // t=<start-time> <stop-time>
    // https://tools.ietf.org/html/rfc4566#section-5.9
    timing: Timing,

    // r=<repeat interval> <active duration> <offsets from start-time>
    // https://tools.ietf.org/html/rfc4566#section-5.10
    repeat_times: Vec<RepeatTime>,
}

// Timing defines the "t=" field's structured representation for the start and
// stop times.
#[derive(Debug, Default)]
pub struct Timing {
    start_time: u64,
    stop_time: u64,
}

impl fmt::Display for Timing {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.start_time, self.stop_time)
    }
}

// RepeatTime describes the "r=" fields of the session description which
// represents the intervals and durations for repeated scheduled sessions.
#[derive(Debug, Default)]
pub struct RepeatTime {
    interval: i64,
    duration: i64,
    offsets: Vec<i64>,
}

impl fmt::Display for RepeatTime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut fields = vec![];
        fields.push(format!("{}", self.interval));
        fields.push(format!("{}", self.duration));
        for value in &self.offsets {
            fields.push(format!("{}", value));
        }
        write!(f, "{}", fields.join(" "))
    }
}

// SessionDescription is a a well-defined format for conveying sufficient
// information to discover and participate in a multimedia session.
#[derive(Debug, Default)]
pub struct SessionDescription {
    // v=0
    // https://tools.ietf.org/html/rfc4566#section-5.1
    pub version: Version,

    // o=<username> <sess-id> <sess-version> <nettype> <addrtype> <unicast-address>
    // https://tools.ietf.org/html/rfc4566#section-5.2
    pub origin: Origin,

    // s=<session name>
    // https://tools.ietf.org/html/rfc4566#section-5.3
    pub session_name: SessionName,

    // i=<session description>
    // https://tools.ietf.org/html/rfc4566#section-5.4
    pub session_information: Option<Information>,

    // u=<uri>
    // https://tools.ietf.org/html/rfc4566#section-5.5
    pub uri: Option<Url>,

    // e=<email-address>
    // https://tools.ietf.org/html/rfc4566#section-5.6
    pub email_address: Option<EmailAddress>,

    // p=<phone-number>
    // https://tools.ietf.org/html/rfc4566#section-5.6
    pub phone_number: Option<PhoneNumber>,

    // c=<nettype> <addrtype> <connection-address>
    // https://tools.ietf.org/html/rfc4566#section-5.7
    pub connection_information: Option<ConnectionInformation>,

    // b=<bwtype>:<bandwidth>
    // https://tools.ietf.org/html/rfc4566#section-5.8
    pub bandwidth: Vec<Bandwidth>,

    // https://tools.ietf.org/html/rfc4566#section-5.9
    // https://tools.ietf.org/html/rfc4566#section-5.10
    pub time_descriptions: Vec<TimeDescription>,

    // z=<adjustment time> <offset> <adjustment time> <offset> ...
    // https://tools.ietf.org/html/rfc4566#section-5.11
    pub time_zones: Vec<TimeZone>,

    // k=<method>
    // k=<method>:<encryption key>
    // https://tools.ietf.org/html/rfc4566#section-5.12
    pub encryption_key: Option<EncryptionKey>,

    // a=<attribute>
    // a=<attribute>:<value>
    // https://tools.ietf.org/html/rfc4566#section-5.13
    pub attributes: Vec<Attribute>,

    // https://tools.ietf.org/html/rfc4566#section-5.14
    pub media_descriptions: Vec<MediaDescription>,
}

// Reset cleans the SessionDescription, and sets all fields back to their default values
impl SessionDescription {
    // API to match draft-ietf-rtcweb-jsep
    // Move to webrtc or its own package?

    // NewJSEPSessionDescription creates a new SessionDescription with
    // some settings that are required by the JSEP spec.
    pub fn new(identity: bool) -> Self {
        let mut d = SessionDescription {
            version: 0,
            origin: Origin {
                username: "-".to_string(),
                session_id: new_session_id(),
                session_version: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .subsec_nanos() as u64,
                network_type: "IN".to_string(),
                address_type: "IP4".to_string(),
                unicast_address: "0.0.0.0".to_string(),
            },
            session_name: "-".to_string(),
            session_information: None,
            uri: None,
            email_address: None,
            phone_number: None,
            connection_information: None,
            bandwidth: vec![],
            time_descriptions: vec![TimeDescription {
                timing: Timing {
                    start_time: 0,
                    stop_time: 0,
                },
                repeat_times: vec![],
            }],
            time_zones: vec![],
            encryption_key: None,
            attributes: vec![], // TODO: implement trickle ICE
            media_descriptions: vec![],
        };

        if identity {
            d.with_property_attribute(ATTR_KEY_IDENTITY.to_string())
        } else {
            d
        }
    }

    // WithPropertyAttribute adds a property attribute 'a=key' to the session description
    pub fn with_property_attribute(mut self, key: String) -> Self {
        self.attributes.push(Attribute::new(key, None));
        self
    }

    // WithValueAttribute adds a value attribute 'a=key:value' to the session description
    pub fn with_value_attribute(mut self, key: String, value: String) -> Self {
        self.attributes.push(Attribute::new(key, Some(value)));
        self
    }

    // WithFingerprint adds a fingerprint to the session description
    pub fn with_fingerprint(mut self, algorithm: String, value: String) -> Self {
        self.with_value_attribute("fingerprint".to_string(), algorithm + " " + value.as_str())
    }

    // WithMedia adds a media description to the session description
    pub fn with_media(mut self, md: MediaDescription) -> Self {
        self.media_descriptions.push(md);
        self
    }

    fn build_codec_map(&self) -> HashMap<u8, Codec> {
        let mut codecs: HashMap<u8, Codec> = HashMap::new();

        for m in &self.media_descriptions {
            for a in &m.attributes {
                let attr = a.to_string();
                if attr.starts_with("rtpmap:") {
                    if let Ok(codec) = parse_rtpmap(&attr) {
                        merge_codecs(codec, &mut codecs);
                    }
                } else if attr.starts_with("fmtp:") {
                    if let Ok(codec) = parse_fmtp(&attr) {
                        merge_codecs(codec, &mut codecs);
                    }
                }
            }
        }

        codecs
    }

    // get_codec_for_payload_type scans the SessionDescription for the given payload type and returns the codec
    pub fn get_codec_for_payload_type(&self, payload_type: u8) -> Result<Codec, Error> {
        let codecs = self.build_codec_map();

        if let Some(codec) = codecs.get(&payload_type) {
            Ok(codec.clone())
        } else {
            Err(Error::new("payload type not found".to_string()))
        }
    }

    // get_payload_type_for_codec scans the SessionDescription for a codec that matches the provided codec
    // as closely as possible and returns its payload type
    pub fn get_payload_type_for_codec(&self, wanted: &Codec) -> Result<u8, Error> {
        let codecs = self.build_codec_map();

        for (payload_type, codec) in codecs.iter() {
            if codecs_match(wanted, codec) {
                return Ok(*payload_type);
            }
        }

        Err(Error::new("codec not found".to_string()))
    }
    // Marshal takes a SDP struct to text
    // https://tools.ietf.org/html/rfc4566#section-5
    // Session description
    //    v=  (protocol version)
    //    o=  (originator and session identifier)
    //    s=  (session name)
    //    i=* (session information)
    //    u=* (URI of description)
    //    e=* (email address)
    //    p=* (phone number)
    //    c=* (connection information -- not required if included in
    //         all media)
    //    b=* (zero or more bandwidth information lines)
    //    One or more time descriptions ("t=" and "r=" lines; see below)
    //    z=* (time zone adjustments)
    //    k=* (encryption key)
    //    a=* (zero or more session attribute lines)
    //    Zero or more media descriptions
    //
    // Time description
    //    t=  (time the session is active)
    //    r=* (zero or more repeat times)
    //
    // Media description, if present
    //    m=  (media name and transport address)
    //    i=* (media title)
    //    c=* (connection information -- optional if included at
    //         session level)
    //    b=* (zero or more bandwidth information lines)
    //    k=* (encryption key)
    //    a=* (zero or more media attribute lines)
    pub fn marshal(&self) -> String {
        let mut result = String::new();

        result += key_value_build("v=", Some(&self.version.to_string())).as_str();
        result += key_value_build("o=", Some(&self.origin.to_string())).as_str();
        result += key_value_build("s=", Some(&self.session_name)).as_str();

        result += key_value_build("i=", self.session_information.as_ref()).as_str();

        if let Some(uri) = &self.uri {
            result += key_value_build("u=", Some(&format!("{}", uri))).as_str();
        }
        result += key_value_build("e=", self.email_address.as_ref()).as_str();
        result += key_value_build("p=", self.phone_number.as_ref()).as_str();
        if let Some(connection_information) = &self.connection_information {
            result += key_value_build("c=", Some(&connection_information.to_string())).as_str();
        }

        for bandwidth in &self.bandwidth {
            result += key_value_build("b=", Some(&bandwidth.to_string())).as_str();
        }
        for time_description in &self.time_descriptions {
            result += key_value_build("t=", Some(&time_description.timing.to_string())).as_str();
            for repeat_time in &time_description.repeat_times {
                result += key_value_build("r=", Some(&repeat_time.to_string())).as_str();
            }
        }
        if self.time_zones.len() > 0 {
            let mut time_zones = vec![];
            for time_zone in &self.time_zones {
                time_zones.push(time_zone.to_string());
            }
            result += key_value_build("z=", Some(&time_zones.join(" "))).as_str();
        }
        result += key_value_build("k=", self.encryption_key.as_ref()).as_str();
        for attribute in &self.attributes {
            result += key_value_build("a=", Some(&attribute.to_string())).as_str();
        }

        for media_description in &self.media_descriptions {
            result +=
                key_value_build("m=", Some(&media_description.media_name.to_string())).as_str();
            result += key_value_build("i=", media_description.media_title.as_ref()).as_str();
            if let Some(connection_information) = &media_description.connection_information {
                result += key_value_build("c=", Some(&connection_information.to_string())).as_str();
            }
            for bandwidth in &media_description.bandwidth {
                result += key_value_build("b=", Some(&bandwidth.to_string())).as_str();
            }
            result += key_value_build("k=", media_description.encryption_key.as_ref()).as_str();
            for attribute in &media_description.attributes {
                result += key_value_build("a=", Some(&attribute.to_string())).as_str();
            }
        }

        result
    }

    // Unmarshal is the primary function that deserializes the session description
    // message and stores it inside of a structured SessionDescription object.
    //
    // The States Transition Table describes the computation flow between functions
    // (namely s1, s2, s3, ...) for a parsing procedure that complies with the
    // specifications laid out by the rfc4566#section-5 as well as by JavaScript
    // Session Establishment Protocol draft. Links:
    // 		https://tools.ietf.org/html/rfc4566#section-5
    // 		https://tools.ietf.org/html/draft-ietf-rtcweb-jsep-24
    //
    // https://tools.ietf.org/html/rfc4566#section-5
    // Session description
    //    v=  (protocol version)
    //    o=  (originator and session identifier)
    //    s=  (session name)
    //    i=* (session information)
    //    u=* (URI of description)
    //    e=* (email address)
    //    p=* (phone number)
    //    c=* (connection information -- not required if included in
    //         all media)
    //    b=* (zero or more bandwidth information lines)
    //    One or more time descriptions ("t=" and "r=" lines; see below)
    //    z=* (time zone adjustments)
    //    k=* (encryption key)
    //    a=* (zero or more session attribute lines)
    //    Zero or more media descriptions
    //
    // Time description
    //    t=  (time the session is active)
    //    r=* (zero or more repeat times)
    //
    // Media description, if present
    //    m=  (media name and transport address)
    //    i=* (media title)
    //    c=* (connection information -- optional if included at
    //         session level)
    //    b=* (zero or more bandwidth information lines)
    //    k=* (encryption key)
    //    a=* (zero or more media attribute lines)
    //
    // In order to generate the following state table and draw subsequent
    // deterministic finite-state automota ("DFA") the following regex was used to
    // derive the DFA:
    // 		vosi?u?e?p?c?b*(tr*)+z?k?a*(mi?c?b*k?a*)*
    //
    // Please pay close attention to the `k`, and `a` parsing states. In the table
    // below in order to distinguish between the states belonging to the media
    // description as opposed to the session description, the states are marked
    // with an asterisk ("a*", "k*").
    // +--------+----+-------+----+-----+----+-----+---+----+----+---+---+-----+---+---+----+---+----+
    // | STATES | a* | a*,k* | a  | a,k | b  | b,c | e | i  | m  | o | p | r,t | s | t | u  | v | z  |
    // +--------+----+-------+----+-----+----+-----+---+----+----+---+---+-----+---+---+----+---+----+
    // |   s1   |    |       |    |     |    |     |   |    |    |   |   |     |   |   |    | 2 |    |
    // |   s2   |    |       |    |     |    |     |   |    |    | 3 |   |     |   |   |    |   |    |
    // |   s3   |    |       |    |     |    |     |   |    |    |   |   |     | 4 |   |    |   |    |
    // |   s4   |    |       |    |     |    |   5 | 6 |  7 |    |   | 8 |     |   | 9 | 10 |   |    |
    // |   s5   |    |       |    |     |  5 |     |   |    |    |   |   |     |   | 9 |    |   |    |
    // |   s6   |    |       |    |     |    |   5 |   |    |    |   | 8 |     |   | 9 |    |   |    |
    // |   s7   |    |       |    |     |    |   5 | 6 |    |    |   | 8 |     |   | 9 | 10 |   |    |
    // |   s8   |    |       |    |     |    |   5 |   |    |    |   |   |     |   | 9 |    |   |    |
    // |   s9   |    |       |    |  11 |    |     |   |    | 12 |   |   |   9 |   |   |    |   | 13 |
    // |   s10  |    |       |    |     |    |   5 | 6 |    |    |   | 8 |     |   | 9 |    |   |    |
    // |   s11  |    |       | 11 |     |    |     |   |    | 12 |   |   |     |   |   |    |   |    |
    // |   s12  |    |    14 |    |     |    |  15 |   | 16 | 12 |   |   |     |   |   |    |   |    |
    // |   s13  |    |       |    |  11 |    |     |   |    | 12 |   |   |     |   |   |    |   |    |
    // |   s14  | 14 |       |    |     |    |     |   |    | 12 |   |   |     |   |   |    |   |    |
    // |   s15  |    |    14 |    |     | 15 |     |   |    | 12 |   |   |     |   |   |    |   |    |
    // |   s16  |    |    14 |    |     |    |  15 |   |    | 12 |   |   |     |   |   |    |   |    |
    // +--------+----+-------+----+-----+----+-----+---+----+----+---+---+-----+---+---+----+---+----+
    pub fn unmarshal<R: io::BufRead>(reader: &mut R) -> Result<Self, Error> {
        let mut lexer = Lexer {
            desc: SessionDescription {
                version: 0,
                origin: Origin::new(),
                session_name: "".to_owned(),
                session_information: None,
                uri: None,
                email_address: None,
                phone_number: None,
                connection_information: None,
                bandwidth: vec![],
                time_descriptions: vec![],
                time_zones: vec![],
                encryption_key: None,
                attributes: vec![],
                media_descriptions: vec![],
            },
            reader,
        };

        let mut state = Some(StateFn { f: s1 });
        while let Some(s) = state {
            state = (s.f)(&mut lexer)?;
        }

        Ok(lexer.desc)
    }
}

fn s1<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, _) = read_type(lexer.reader)?;
    if &key == "v=" {
        return Ok(Some(StateFn {
            f: unmarshal_protocol_version,
        }));
    }

    Err(Error::new(format!("sdp: invalid syntax `{}`", key)))
}

fn s2<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, _) = read_type(lexer.reader)?;
    if &key == "o=" {
        return Ok(Some(StateFn {
            f: unmarshal_origin,
        }));
    }

    Err(Error::new(format!("sdp: invalid syntax `{}`", key)))
}

fn s3<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, _) = read_type(lexer.reader)?;
    if &key == "s=" {
        return Ok(Some(StateFn {
            f: unmarshal_session_name,
        }));
    }

    Err(Error::new(format!("sdp: invalid syntax `{}`", key)))
}

fn s4<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, _) = read_type(lexer.reader)?;
    match key.as_str() {
        "i=" => Ok(Some(StateFn {
            f: unmarshal_session_information,
        })),
        "u=" => Ok(Some(StateFn { f: unmarshal_uri })),
        "e=" => Ok(Some(StateFn { f: unmarshal_email })),
        "p=" => Ok(Some(StateFn { f: unmarshal_phone })),
        "c=" => Ok(Some(StateFn {
            f: unmarshal_session_connection_information,
        })),
        "b=" => Ok(Some(StateFn {
            f: unmarshal_session_bandwidth,
        })),
        "t=" => Ok(Some(StateFn {
            f: unmarshal_timing,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s5<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, _) = read_type(lexer.reader)?;
    match key.as_str() {
        "b=" => Ok(Some(StateFn {
            f: unmarshal_session_bandwidth,
        })),
        "t=" => Ok(Some(StateFn {
            f: unmarshal_timing,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s6<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, _) = read_type(lexer.reader)?;
    match key.as_str() {
        "p=" => Ok(Some(StateFn { f: unmarshal_phone })),
        "c=" => Ok(Some(StateFn {
            f: unmarshal_session_connection_information,
        })),
        "b=" => Ok(Some(StateFn {
            f: unmarshal_session_bandwidth,
        })),
        "t=" => Ok(Some(StateFn {
            f: unmarshal_timing,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s7<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, _) = read_type(lexer.reader)?;
    match key.as_str() {
        "u=" => Ok(Some(StateFn { f: unmarshal_uri })),
        "e=" => Ok(Some(StateFn { f: unmarshal_email })),
        "p=" => Ok(Some(StateFn { f: unmarshal_phone })),
        "c=" => Ok(Some(StateFn {
            f: unmarshal_session_connection_information,
        })),
        "b=" => Ok(Some(StateFn {
            f: unmarshal_session_bandwidth,
        })),
        "t=" => Ok(Some(StateFn {
            f: unmarshal_timing,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s8<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, _) = read_type(lexer.reader)?;
    match key.as_str() {
        "c=" => Ok(Some(StateFn {
            f: unmarshal_session_connection_information,
        })),
        "b=" => Ok(Some(StateFn {
            f: unmarshal_session_bandwidth,
        })),
        "t=" => Ok(Some(StateFn {
            f: unmarshal_timing,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s9<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, num_bytes) = read_type(lexer.reader)?;
    if &key == "" && num_bytes == 0 {
        return Ok(None);
    }

    match key.as_str() {
        "z=" => Ok(Some(StateFn {
            f: unmarshal_time_zones,
        })),
        "k=" => Ok(Some(StateFn {
            f: unmarshal_session_encryption_key,
        })),
        "a=" => Ok(Some(StateFn {
            f: unmarshal_session_attribute,
        })),
        "r=" => Ok(Some(StateFn {
            f: unmarshal_repeat_times,
        })),
        "t=" => Ok(Some(StateFn {
            f: unmarshal_timing,
        })),
        "m=" => Ok(Some(StateFn {
            f: unmarshal_media_description,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s10<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, _) = read_type(lexer.reader)?;
    match key.as_str() {
        "e=" => Ok(Some(StateFn { f: unmarshal_email })),
        "p=" => Ok(Some(StateFn { f: unmarshal_phone })),
        "c=" => Ok(Some(StateFn {
            f: unmarshal_session_connection_information,
        })),
        "b=" => Ok(Some(StateFn {
            f: unmarshal_session_bandwidth,
        })),
        "t=" => Ok(Some(StateFn {
            f: unmarshal_timing,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s11<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, num_bytes) = read_type(lexer.reader)?;
    if &key == "" && num_bytes == 0 {
        return Ok(None);
    }

    match key.as_str() {
        "a=" => Ok(Some(StateFn {
            f: unmarshal_session_attribute,
        })),
        "m=" => Ok(Some(StateFn {
            f: unmarshal_media_description,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s12<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, num_bytes) = read_type(lexer.reader)?;
    if &key == "" && num_bytes == 0 {
        return Ok(None);
    }

    match key.as_str() {
        "a=" => Ok(Some(StateFn {
            f: unmarshal_media_attribute,
        })),
        "k=" => Ok(Some(StateFn {
            f: unmarshal_media_encryption_key,
        })),
        "b=" => Ok(Some(StateFn {
            f: unmarshal_media_bandwidth,
        })),
        "c=" => Ok(Some(StateFn {
            f: unmarshal_media_connection_information,
        })),
        "i=" => Ok(Some(StateFn {
            f: unmarshal_media_title,
        })),
        "m=" => Ok(Some(StateFn {
            f: unmarshal_media_description,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s13<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, num_bytes) = read_type(lexer.reader)?;
    if &key == "" && num_bytes == 0 {
        return Ok(None);
    }

    match key.as_str() {
        "a=" => Ok(Some(StateFn {
            f: unmarshal_session_attribute,
        })),
        "k=" => Ok(Some(StateFn {
            f: unmarshal_session_encryption_key,
        })),
        "m=" => Ok(Some(StateFn {
            f: unmarshal_media_description,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s14<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, num_bytes) = read_type(lexer.reader)?;
    if &key == "" && num_bytes == 0 {
        return Ok(None);
    }

    match key.as_str() {
        "a=" => Ok(Some(StateFn {
            f: unmarshal_media_attribute,
        })),
        "m=" => Ok(Some(StateFn {
            f: unmarshal_media_description,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s15<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, num_bytes) = read_type(lexer.reader)?;
    if &key == "" && num_bytes == 0 {
        return Ok(None);
    }

    match key.as_str() {
        "a=" => Ok(Some(StateFn {
            f: unmarshal_media_attribute,
        })),
        "k=" => Ok(Some(StateFn {
            f: unmarshal_media_encryption_key,
        })),
        "b=" => Ok(Some(StateFn {
            f: unmarshal_media_bandwidth,
        })),
        "m=" => Ok(Some(StateFn {
            f: unmarshal_media_description,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn s16<'a, R: io::BufRead>(lexer: &mut Lexer<'a, R>) -> Result<Option<StateFn<'a, R>>, Error> {
    let (key, num_bytes) = read_type(lexer.reader)?;
    if &key == "" && num_bytes == 0 {
        return Ok(None);
    }

    match key.as_str() {
        "a=" => Ok(Some(StateFn {
            f: unmarshal_media_attribute,
        })),
        "k=" => Ok(Some(StateFn {
            f: unmarshal_media_encryption_key,
        })),
        "c=" => Ok(Some(StateFn {
            f: unmarshal_media_connection_information,
        })),
        "b=" => Ok(Some(StateFn {
            f: unmarshal_media_bandwidth,
        })),
        "m=" => Ok(Some(StateFn {
            f: unmarshal_media_description,
        })),
        _ => Err(Error::new(format!("sdp: invalid syntax `{}`", key))),
    }
}

fn unmarshal_protocol_version<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    let version = value.parse::<u32>()?;

    // As off the latest draft of the rfc this value is required to be 0.
    // https://tools.ietf.org/html/draft-ietf-rtcweb-jsep-24#section-5.8.1
    if version != 0 {
        return Err(Error::new(format!("sdp: invalid value `{}`", version)));
    }

    Ok(Some(StateFn { f: s2 }))
}

fn unmarshal_origin<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    let fields: Vec<&str> = value.split_whitespace().collect();
    if fields.len() != 6 {
        return Err(Error::new(format!("sdp: invalid syntax `o={}`", value)));
    }

    let session_id = fields[1].parse::<u64>()?;
    let session_version = fields[2].parse::<u64>()?;

    // Set according to currently registered with IANA
    // https://tools.ietf.org/html/rfc4566#section-8.2.6
    let i = index_of(fields[3], &vec!["IN"]);
    if i == -1 {
        return Err(Error::new(format!("sdp: invalid value `{}`", fields[3])));
    }

    // Set according to currently registered with IANA
    // https://tools.ietf.org/html/rfc4566#section-8.2.7
    let i = index_of(fields[4], &vec!["IP4", "IP6"]);
    if i == -1 {
        return Err(Error::new(format!("sdp: invalid value `{}`", fields[4])));
    }

    // TODO validated UnicastAddress

    lexer.desc.origin = Origin {
        username: fields[0].to_owned(),
        session_id,
        session_version,
        network_type: fields[3].to_owned(),
        address_type: fields[4].to_owned(),
        unicast_address: fields[5].to_owned(),
    };

    Ok(Some(StateFn { f: s3 }))
}

fn unmarshal_session_name<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;
    lexer.desc.session_name = value;
    Ok(Some(StateFn { f: s4 }))
}

fn unmarshal_session_information<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;
    lexer.desc.session_information = Some(value);
    Ok(Some(StateFn { f: s7 }))
}

fn unmarshal_uri<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;
    lexer.desc.uri = Some(Url::parse(&value)?);
    Ok(Some(StateFn { f: s10 }))
}

fn unmarshal_email<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;
    lexer.desc.email_address = Some(value);
    Ok(Some(StateFn { f: s6 }))
}

fn unmarshal_phone<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;
    lexer.desc.phone_number = Some(value);
    Ok(Some(StateFn { f: s8 }))
}

fn unmarshal_session_connection_information<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;
    lexer.desc.connection_information = unmarshal_connection_information(&value)?;
    Ok(Some(StateFn { f: s5 }))
}

fn unmarshal_connection_information(value: &str) -> Result<Option<ConnectionInformation>, Error> {
    let fields: Vec<&str> = value.split_whitespace().collect();
    if fields.len() < 2 {
        return Err(Error::new(format!("sdp: invalid syntax `c={}`", value)));
    }

    // Set according to currently registered with IANA
    // https://tools.ietf.org/html/rfc4566#section-8.2.6
    let i = index_of(fields[0], &vec!["IN"]);
    if i == -1 {
        return Err(Error::new(format!("sdp: invalid value `{}`", fields[0])));
    }

    // Set according to currently registered with IANA
    // https://tools.ietf.org/html/rfc4566#section-8.2.7
    let i = index_of(fields[1], &vec!["IP4", "IP6"]);
    if i == -1 {
        return Err(Error::new(format!("sdp: invalid value `{}`", fields[1])));
    }

    let address = if fields.len() > 2 {
        Some(Address {
            address: fields[2].to_owned(),
            ttl: None,
            range: None,
        })
    } else {
        None
    };

    Ok(Some(ConnectionInformation {
        network_type: fields[0].to_owned(),
        address_type: fields[1].to_owned(),
        address,
    }))
}

fn unmarshal_session_bandwidth<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;
    lexer.desc.bandwidth.push(unmarshal_bandwidth(&value)?);
    Ok(Some(StateFn { f: s5 }))
}

fn unmarshal_bandwidth(value: &str) -> Result<Bandwidth, Error> {
    let mut parts: Vec<&str> = value.split(":").collect();
    if parts.len() != 2 {
        return Err(Error::new(format!("sdp: invalid syntax `b={}`", value)));
    }

    let experimental = parts[0].starts_with("X-");
    if experimental {
        parts[0] = parts[0].trim_start_matches("X-");
    } else {
        // Set according to currently registered with IANA
        // https://tools.ietf.org/html/rfc4566#section-5.8
        let i = index_of(parts[0], &vec!["CT", "AS"]);
        if i == -1 {
            return Err(Error::new(format!("sdp: invalid value `{}`", parts[0])));
        }
    }

    let bandwidth = parts[1].parse::<u64>()?;

    Ok(Bandwidth {
        experimental,
        bandwidth_type: parts[0].to_owned(),
        bandwidth,
    })
}

fn unmarshal_timing<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    let fields: Vec<&str> = value.split_whitespace().collect();
    if fields.len() < 2 {
        return Err(Error::new(format!("sdp: invalid syntax `t={}`", value)));
    }

    let start_time = fields[0].parse::<u64>()?;
    let stop_time = fields[1].parse::<u64>()?;

    lexer.desc.time_descriptions.push(TimeDescription {
        timing: Timing {
            start_time,
            stop_time,
        },
        repeat_times: vec![],
    });

    Ok(Some(StateFn { f: s9 }))
}

fn unmarshal_repeat_times<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    let fields: Vec<&str> = value.split_whitespace().collect();
    if fields.len() < 3 {
        return Err(Error::new(format!("sdp: invalid syntax `r={}`", value)));
    }

    if let Some(latest_time_desc) = lexer.desc.time_descriptions.last_mut() {
        let interval = parse_time_units(fields[0])?;
        let duration = parse_time_units(fields[1])?;
        let mut offsets = vec![];
        for i in 2..fields.len() {
            let offset = parse_time_units(fields[i])?;
            offsets.push(offset);
        }
        latest_time_desc.repeat_times.push(RepeatTime {
            interval,
            duration,
            offsets,
        });

        Ok(Some(StateFn { f: s9 }))
    } else {
        Err(Error::new(format!("sdp: empty time_descriptions")))
    }
}

fn unmarshal_time_zones<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    // These fields are transimitted in pairs
    // z=<adjustment time> <offset> <adjustment time> <offset> ....
    // so we are making sure that there are actually multiple of 2 total.
    let fields: Vec<&str> = value.split_whitespace().collect();
    if fields.len() % 2 != 0 {
        return Err(Error::new(format!("sdp: invalid syntax `t={}`", value)));
    }

    for i in (0..fields.len()).step_by(2) {
        let adjustment_time = fields[i].parse::<u64>()?;
        let offset = parse_time_units(fields[i + 1])?;

        lexer.desc.time_zones.push(TimeZone {
            adjustment_time,
            offset,
        });
    }

    Ok(Some(StateFn { f: s13 }))
}

fn unmarshal_session_encryption_key<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;
    lexer.desc.encryption_key = Some(value);
    Ok(Some(StateFn { f: s11 }))
}

fn unmarshal_session_attribute<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    let fields: Vec<&str> = value.splitn(2, ':').collect();
    let attribute = if fields.len() == 2 {
        Attribute {
            key: fields[0].to_owned(),
            value: Some(fields[1].to_owned()),
        }
    } else {
        Attribute {
            key: fields[0].to_owned(),
            value: None,
        }
    };
    lexer.desc.attributes.push(attribute);

    Ok(Some(StateFn { f: s11 }))
}

fn unmarshal_media_description<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    let fields: Vec<&str> = value.split_whitespace().collect();
    if fields.len() < 4 {
        return Err(Error::new(format!("sdp: invalid syntax `m={}`", value)));
    }

    // <media>
    // Set according to currently registered with IANA
    // https://tools.ietf.org/html/rfc4566#section-5.14
    let i = index_of(
        fields[0],
        &vec!["audio", "video", "text", "application", "message"],
    );
    if i == -1 {
        return Err(Error::new(format!("sdp: invalid value `{}`", fields[0])));
    }

    // <port>
    let parts: Vec<&str> = fields[1].split("/").collect();
    let port_value = parts[0].parse::<u16>()? as isize;
    let port_range = if parts.len() > 1 {
        Some(parts[1].parse::<i32>()? as isize)
    } else {
        None
    };

    // <proto>
    // Set according to currently registered with IANA
    // https://tools.ietf.org/html/rfc4566#section-5.14
    let mut protos = vec![];
    for proto in fields[2].split("/").collect::<Vec<&str>>() {
        let i = index_of(
            proto,
            &vec!["UDP", "RTP", "AVP", "SAVP", "SAVPF", "TLS", "DTLS", "SCTP"],
        );
        if i == -1 {
            return Err(Error::new(format!("sdp: invalid value `{}`", fields[2])));
        }
        protos.push(proto.to_owned());
    }

    // <fmt>...
    let mut formats = vec![];
    for i in 3..fields.len() {
        formats.push(fields[i].to_owned());
    }

    lexer.desc.media_descriptions.push(MediaDescription {
        media_name: MediaName {
            media: fields[0].to_owned(),
            port: RangedPort {
                value: port_value,
                range: port_range,
            },
            protos,
            formats,
        },
        media_title: None,
        connection_information: None,
        bandwidth: vec![],
        encryption_key: None,
        attributes: vec![],
    });

    Ok(Some(StateFn { f: s12 }))
}

fn unmarshal_media_title<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    if let Some(latest_media_desc) = lexer.desc.media_descriptions.last_mut() {
        latest_media_desc.media_title = Some(value);
        Ok(Some(StateFn { f: s16 }))
    } else {
        Err(Error::new(format!("sdp: empty media_descriptions")))
    }
}

fn unmarshal_media_connection_information<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    if let Some(latest_media_desc) = lexer.desc.media_descriptions.last_mut() {
        latest_media_desc.connection_information = unmarshal_connection_information(&value)?;
        Ok(Some(StateFn { f: s15 }))
    } else {
        Err(Error::new(format!("sdp: empty media_descriptions")))
    }
}

fn unmarshal_media_bandwidth<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    if let Some(latest_media_desc) = lexer.desc.media_descriptions.last_mut() {
        let bandwidth = unmarshal_bandwidth(&value)?;
        latest_media_desc.bandwidth.push(bandwidth);
        Ok(Some(StateFn { f: s15 }))
    } else {
        Err(Error::new(format!("sdp: empty media_descriptions")))
    }
}

fn unmarshal_media_encryption_key<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    if let Some(latest_media_desc) = lexer.desc.media_descriptions.last_mut() {
        latest_media_desc.encryption_key = Some(value);
        Ok(Some(StateFn { f: s14 }))
    } else {
        Err(Error::new(format!("sdp: empty media_descriptions")))
    }
}

fn unmarshal_media_attribute<'a, R: io::BufRead>(
    lexer: &mut Lexer<'a, R>,
) -> Result<Option<StateFn<'a, R>>, Error> {
    let (value, _) = read_value(lexer.reader)?;

    let fields: Vec<&str> = value.splitn(2, ':').collect();
    let attribute = if fields.len() == 2 {
        Attribute {
            key: fields[0].to_owned(),
            value: Some(fields[1].to_owned()),
        }
    } else {
        Attribute {
            key: fields[0].to_owned(),
            value: None,
        }
    };

    if let Some(latest_media_desc) = lexer.desc.media_descriptions.last_mut() {
        latest_media_desc.attributes.push(attribute);
        Ok(Some(StateFn { f: s14 }))
    } else {
        Err(Error::new(format!("sdp: empty media_descriptions")))
    }
}

fn parse_time_units(value: &str) -> Result<i64, Error> {
    // Some time offsets in the protocol can be provided with a shorthand
    // notation. This code ensures to convert it to NTP timestamp format.
    //      d - days (86400 seconds)
    //      h - hours (3600 seconds)
    //      m - minutes (60 seconds)
    //      s - seconds (allowed for completeness)
    let val = value.as_bytes();
    let len = val.len();
    let num = match val[len - 1] {
        b'd' => value.trim_end_matches("d").parse::<i64>()? * 86400,
        b'h' => value.trim_end_matches("h").parse::<i64>()? * 3600,
        b'm' => value.trim_end_matches("m").parse::<i64>()? * 60,
        _ => value.trim_end_matches("m").parse::<i64>()?,
    };

    Ok(num)
}
