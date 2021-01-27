use util::Error;

lazy_static! {
    pub static ref ERR_RELAY_ADDRESS_INVALID: Error = Error::new(
        "turn: RelayAddress must be valid IP to use RelayAddressGeneratorStatic".to_owned()
    );
    pub static ref ERR_NO_AVAILABLE_CONNS: Error = Error::new(
        "turn: PacketConnConfigs and ConnConfigs are empty, unable to proceed".to_owned()
    );
    pub static ref ERR_CONN_UNSET: Error =
        Error::new("turn: PacketConnConfig must have a non-nil Conn".to_owned());
    pub static ref ERR_LISTENER_UNSET: Error =
        Error::new("turn: ListenerConfig must have a non-nil Listener".to_owned());
    pub static ref ERR_LISTENING_ADDRESS_INVALID: Error =
        Error::new("turn: RelayAddressGenerator has invalid ListeningAddress".to_owned());
    pub static ref ERR_RELAY_ADDRESS_GENERATOR_UNSET: Error =
        Error::new("turn: RelayAddressGenerator in RelayConfig is unset".to_owned());
    pub static ref ERR_MAX_RETRIES_EXCEEDED: Error =
        Error::new("turn: max retries exceeded".to_owned());
    pub static ref ERR_MAX_PORT_NOT_ZERO: Error =
        Error::new("turn: MaxPort must be not 0".to_owned());
    pub static ref ERR_MIN_PORT_NOT_ZERO: Error =
        Error::new("turn: MaxPort must be not 0".to_owned());
    pub static ref ERR_MAX_PORT_LESS_THAN_MIN_PORT: Error =
        Error::new("turn: MaxPort less than MinPort".to_owned());
    pub static ref ERR_NIL_CONN: Error = Error::new("turn: relay_conn cannot not be nil".to_owned());
    pub static ref ERR_TODO: Error = Error::new("turn: TODO".to_owned());
    pub static ref ERR_ALREADY_LISTENING: Error = Error::new("turn: already listening".to_owned());
    pub static ref ERR_FAILED_TO_CLOSE: Error =
        Error::new("turn: Server failed to close".to_owned());
    pub static ref ERR_FAILED_TO_RETRANSMIT_TRANSACTION: Error =
        Error::new("turn: failed to retransmit transaction".to_owned());
    pub static ref ERR_ALL_RETRANSMISSIONS_FAILED: Error =
        Error::new("all retransmissions failed for".to_owned());
    pub static ref ERR_CHANNEL_BIND_NOT_FOUND: Error =
        Error::new("no binding found for channel".to_owned());
    pub static ref ERR_STUNSERVER_ADDRESS_NOT_SET: Error =
        Error::new("STUN server address is not set for the client".to_owned());
    pub static ref ERR_ONE_ALLOCATE_ONLY: Error =
        Error::new("only one Allocate() caller is allowed".to_owned());
    pub static ref ERR_ALREADY_ALLOCATED: Error = Error::new("already allocated".to_owned());
    pub static ref ERR_NON_STUNMESSAGE: Error =
        Error::new("non-STUN message from STUN server".to_owned());
    pub static ref ERR_FAILED_TO_DECODE_STUN: Error =
        Error::new("failed to decode STUN message".to_owned());
    pub static ref ERR_UNEXPECTED_STUNREQUEST_MESSAGE: Error =
        Error::new("unexpected STUN request message".to_owned());

    // ErrInvalidChannelNumber means that channel number is not valid as by RFC 5766 Section 11.
    pub static ref ERR_INVALID_CHANNEL_NUMBER: Error =
        Error::new("channel number not in [0x4000, 0x7FFF]".to_owned());
    // ErrBadChannelDataLength means that channel data length is not equal
    // to actual data length.
    pub static ref ERR_BAD_CHANNEL_DATA_LENGTH: Error =
        Error::new("channelData length != len(Data)".to_owned());
    pub static ref ERR_UNEXPECTED_EOF: Error = Error::new("unexpected EOF".to_owned());
    pub static ref ERR_INVALID_REQUESTED_FAMILY_VALUE: Error = Error::new("invalid value for requested family attribute".to_owned());

    pub static ref ERR_FAKE_ERR: Error = Error::new("fake error".to_owned());
    pub static ref ERR_TRY_AGAIN: Error = Error::new("try again".to_owned());
    pub static ref ERR_CLOSED: Error = Error::new("use of closed network connection".to_owned());
    pub static ref ERR_UDPADDR_CAST: Error = Error::new("addr is not a net.UDPAddr".to_owned());
    pub static ref ERR_ALREADY_CLOSED: Error = Error::new("already closed".to_owned());
    pub static ref ERR_DOUBLE_LOCK: Error = Error::new("try-lock is already locked".to_owned());
    pub static ref ERR_TRANSACTION_CLOSED: Error = Error::new("transaction closed".to_owned());
    pub static ref ERR_WAIT_FOR_RESULT_ON_NON_RESULT_TRANSACTION: Error = Error::new("wait_for_result called on non-result transaction".to_owned());
    pub static ref ERR_FAILED_TO_BUILD_REFRESH_REQUEST: Error = Error::new("failed to build refresh request".to_owned());
    pub static ref ERR_FAILED_TO_REFRESH_ALLOCATION: Error = Error::new("failed to refresh allocation".to_owned());
    pub static ref ERR_FAILED_TO_GET_LIFETIME: Error = Error::new("failed to get lifetime from refresh response".to_owned());
    pub static ref ERR_SHORT_BUFFER: Error = Error::new("too short buffer".to_owned());
    pub static ref ERR_UNEXPECTED_RESPONSE: Error = Error::new("unexpected response type".to_owned());

    pub static ref ERR_ALLOCATE_PACKET_CONN_MUST_BE_SET: Error = Error::new("AllocatePacketConn must be set".to_owned());
    pub static ref ERR_ALLOCATE_CONN_MUST_BE_SET: Error = Error::new("AllocateConn must be set".to_owned());
    pub static ref ERR_LEVELED_LOGGER_MUST_BE_SET: Error = Error::new("LeveledLogger must be set".to_owned());
    pub static ref ERR_SAME_CHANNEL_DIFFERENT_PEER: Error = Error::new("you cannot use the same channel number with different peer".to_owned());
    pub static ref ERR_NIL_FIVE_TUPLE: Error = Error::new("allocations must not be created with nil FivTuple".to_owned());
    pub static ref ERR_NIL_FIVE_TUPLE_SRC_ADDR: Error = Error::new("allocations must not be created with nil FiveTuple.src_addr".to_owned());
    pub static ref ERR_NIL_FIVE_TUPLE_DST_ADDR: Error = Error::new("allocations must not be created with nil FiveTuple.dst_addr".to_owned());
    pub static ref ERR_NIL_TURN_SOCKET: Error = Error::new("allocations must not be created with nil turnSocket".to_owned());
    pub static ref ERR_LIFETIME_ZERO: Error = Error::new("allocations must not be created with a lifetime of 0".to_owned());
    pub static ref ERR_DUPE_FIVE_TUPLE: Error = Error::new("allocation attempt created with duplicate FiveTuple".to_owned());
    pub static ref ERR_FAILED_TO_CAST_UDPADDR: Error = Error::new("failed to cast net.Addr to *net.UDPAddr".to_owned());

    pub static ref ERR_FAILED_TO_GENERATE_NONCE: Error = Error::new("failed to generate nonce".to_owned());
    pub static ref ERR_FAILED_TO_SEND_ERROR: Error = Error::new("failed to send error message".to_owned());
    pub static ref ERR_DUPLICATED_NONCE: Error = Error::new("duplicated Nonce generated, discarding request".to_owned());
    pub static ref ERR_NO_SUCH_USER: Error = Error::new("no such user exists".to_owned());
    pub static ref ERR_UNEXPECTED_CLASS: Error = Error::new("unexpected class".to_owned());
    pub static ref ERR_UNEXPECTED_METHOD: Error = Error::new("unexpected method".to_owned());
    pub static ref ERR_FAILED_TO_HANDLE: Error = Error::new("failed to handle".to_owned());
    pub static ref ERR_UNHANDLED_STUNPACKET: Error = Error::new("unhandled STUN packet".to_owned());
    pub static ref ERR_UNABLE_TO_HANDLE_CHANNEL_DATA: Error = Error::new("unable to handle ChannelData".to_owned());
    pub static ref ERR_FAILED_TO_CREATE_STUNPACKET: Error = Error::new("failed to create stun message from packet".to_owned());
    pub static ref ERR_FAILED_TO_CREATE_CHANNEL_DATA: Error = Error::new("failed to create channel data from packet".to_owned());
    pub static ref ERR_RELAY_ALREADY_ALLOCATED_FOR_FIVE_TUPLE: Error = Error::new("relay already allocated for 5-TUPLE".to_owned());
    pub static ref ERR_REQUESTED_TRANSPORT_MUST_BE_UDP: Error = Error::new("RequestedTransport must be UDP".to_owned());
    pub static ref ERR_NO_DONT_FRAGMENT_SUPPORT: Error = Error::new("no support for DONT-FRAGMENT".to_owned());
    pub static ref ERR_REQUEST_WITH_RESERVATION_TOKEN_AND_EVEN_PORT: Error = Error::new("Request must not contain RESERVATION-TOKEN and EVEN-PORT".to_owned());
    pub static ref ERR_NO_ALLOCATION_FOUND: Error = Error::new("no allocation found".to_owned());
    pub static ref ERR_NO_PERMISSION: Error = Error::new("unable to handle send-indication, no permission added".to_owned());
    pub static ref ERR_SHORT_WRITE: Error = Error::new("packet write smaller than packet".to_owned());
    pub static ref ERR_NO_SUCH_CHANNEL_BIND: Error = Error::new("no such channel bind".to_owned());
    pub static ref ERR_FAILED_WRITE_SOCKET: Error = Error::new("failed writing to socket".to_owned());

}
