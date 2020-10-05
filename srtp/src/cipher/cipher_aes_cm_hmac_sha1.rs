use aes;
use ctr;
use ctr::stream_cipher::generic_array::GenericArray;
use ctr::stream_cipher::{NewStreamCipher, StreamCipher};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;

use super::*;
use crate::context::*;
use crate::key_derivation::*;
use crate::protection_profile::*;

use util::Error;

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{BufWriter, Cursor};

type HmacSha1 = Hmac<Sha1>;
type Aes128Ctr = ctr::Ctr128<aes::Aes128>;

pub(crate) const CIPHER_AES_CM_HMAC_SHA1AUTH_TAG_LEN: usize = 10;

pub(crate) struct CipherAesCmHmacSha1 {
    srtp_session_key: Vec<u8>,
    srtp_session_salt: Vec<u8>,
    srtp_session_auth: HmacSha1,
    srtp_session_auth_tag: Vec<u8>,

    srtcp_session_key: Vec<u8>,
    srtcp_session_salt: Vec<u8>,
    srtcp_session_auth: HmacSha1,
    srtcp_session_auth_tag: Vec<u8>,
}

impl CipherAesCmHmacSha1 {
    pub fn new(master_key: &[u8], master_salt: &[u8]) -> Result<Self, Error> {
        let srtp_session_key = aes_cm_key_derivation(
            LABEL_SRTP_ENCRYPTION,
            master_key,
            master_salt,
            0,
            master_key.len(),
        )?;
        let srtcp_session_key = aes_cm_key_derivation(
            LABEL_SRTCP_ENCRYPTION,
            master_key,
            master_salt,
            0,
            master_key.len(),
        )?;

        let srtp_session_salt = aes_cm_key_derivation(
            LABEL_SRTP_SALT,
            master_key,
            master_salt,
            0,
            master_salt.len(),
        )?;
        let srtcp_session_salt = aes_cm_key_derivation(
            LABEL_SRTCP_SALT,
            master_key,
            master_salt,
            0,
            master_salt.len(),
        )?;

        let auth_key_len = PROTECTION_PROFILE_AES128CM_HMAC_SHA1_80.auth_key_len()?;

        let srtp_session_auth_tag = aes_cm_key_derivation(
            LABEL_SRTP_AUTHENTICATION_TAG,
            master_key,
            master_salt,
            0,
            auth_key_len,
        )?;
        let srtcp_session_auth_tag = aes_cm_key_derivation(
            LABEL_SRTCP_AUTHENTICATION_TAG,
            master_key,
            master_salt,
            0,
            auth_key_len,
        )?;

        let srtp_session_auth = match HmacSha1::new_varkey(&srtp_session_auth_tag) {
            Ok(srtp_session_auth) => srtp_session_auth,
            Err(err) => return Err(Error::new(err.to_string())),
        };
        let srtcp_session_auth = match HmacSha1::new_varkey(&srtcp_session_auth_tag) {
            Ok(srtcp_session_auth) => srtcp_session_auth,
            Err(err) => return Err(Error::new(err.to_string())),
        };

        Ok(CipherAesCmHmacSha1 {
            srtp_session_key,
            srtp_session_salt,
            srtp_session_auth,
            srtp_session_auth_tag,

            srtcp_session_key,
            srtcp_session_salt,
            srtcp_session_auth,
            srtcp_session_auth_tag,
        })
    }

    fn generate_srtp_auth_tag(&mut self, buf: &[u8], roc: u32) -> Result<Vec<u8>, Error> {
        // https://tools.ietf.org/html/rfc3711#section-4.2
        // In the case of SRTP, M SHALL consist of the Authenticated
        // Portion of the packet (as specified in Figure 1) concatenated with
        // the ROC, M = Authenticated Portion || ROC;
        //
        // The pre-defined authentication transform for SRTP is HMAC-SHA1
        // [RFC2104].  With HMAC-SHA1, the SRTP_PREFIX_LENGTH (Figure 3) SHALL
        // be 0.  For SRTP (respectively SRTCP), the HMAC SHALL be applied to
        // the session authentication key and M as specified above, i.e.,
        // HMAC(k_a, M).  The HMAC output SHALL then be truncated to the n_tag
        // left-most bits.
        // - Authenticated portion of the packet is everything BEFORE MKI
        // - k_a is the session message authentication key
        // - n_tag is the bit-length of the output authentication tag
        self.srtp_session_auth.reset();

        self.srtp_session_auth.input(buf);

        // For SRTP only, we need to hash the rollover counter as well.
        let mut roc_buf: Vec<u8> = vec![];
        {
            let mut writer = BufWriter::<&mut Vec<u8>>::new(roc_buf.as_mut());
            writer.write_u32::<BigEndian>(roc)?;
        }

        self.srtp_session_auth.input(&roc_buf);

        let result = self.srtp_session_auth.clone().result();
        let code_bytes = result.code();

        // Truncate the hash to the first AUTH_TAG_SIZE bytes.
        Ok(code_bytes[0..self.auth_tag_len()].to_vec())
    }

    fn generate_srtcp_auth_tag(&mut self, buf: &[u8]) -> Result<Vec<u8>, Error> {
        // https://tools.ietf.org/html/rfc3711#section-4.2
        //
        // The pre-defined authentication transform for SRTP is HMAC-SHA1
        // [RFC2104].  With HMAC-SHA1, the SRTP_PREFIX_LENGTH (Figure 3) SHALL
        // be 0.  For SRTP (respectively SRTCP), the HMAC SHALL be applied to
        // the session authentication key and M as specified above, i.e.,
        // HMAC(k_a, M).  The HMAC output SHALL then be truncated to the n_tag
        // left-most bits.
        // - Authenticated portion of the packet is everything BEFORE MKI
        // - k_a is the session message authentication key
        // - n_tag is the bit-length of the output authentication tag
        self.srtcp_session_auth.reset();

        self.srtcp_session_auth.input(buf);

        let result = self.srtcp_session_auth.clone().result();
        let code_bytes = result.code();

        // Truncate the hash to the first AUTH_TAG_SIZE bytes.
        Ok(code_bytes[0..self.auth_tag_len()].to_vec())
    }
}

impl Cipher for CipherAesCmHmacSha1 {
    fn auth_tag_len(&self) -> usize {
        CIPHER_AES_CM_HMAC_SHA1AUTH_TAG_LEN
    }

    fn get_rtcp_index(&self, input: &[u8]) -> Result<u32, Error> {
        let tail_offset = input.len() - (self.auth_tag_len() + SRTCP_INDEX_SIZE);
        let mut reader = Cursor::new(&input[tail_offset..tail_offset + SRTCP_INDEX_SIZE]);
        let rtcp_index = reader.read_u32::<BigEndian>()? & 0x7FFFFFFF; //^(1 << 31)
        Ok(rtcp_index)
    }

    fn encrypt_rtp(
        &mut self,
        header: &rtp::header::Header,
        payload: &[u8],
        roc: u32,
    ) -> Result<Vec<u8>, Error> {
        let mut dst: Vec<u8> =
            Vec::with_capacity(header.len() + payload.len() + self.auth_tag_len());

        // Copy the header unencrypted.
        {
            let mut writer = BufWriter::<&mut Vec<u8>>::new(dst.as_mut());
            header.marshal(&mut writer)?;
        }

        // Write the plaintext header to the destination buffer.
        dst.extend_from_slice(payload);

        // Encrypt the payload
        let counter = generate_counter(
            header.sequence_number,
            roc,
            header.ssrc,
            &self.srtp_session_salt,
        )?;
        let key = GenericArray::from_slice(&self.srtp_session_key);
        let nonce = GenericArray::from_slice(&counter);
        let mut stream = Aes128Ctr::new(&key, &nonce);

        stream.encrypt(&mut dst[header.payload_offset..]);

        // Generate the auth tag.
        let auth_tag = self.generate_srtp_auth_tag(&dst, roc)?;

        dst.extend_from_slice(&auth_tag);

        Ok(dst)
    }

    fn decrypt_rtp(
        &mut self,
        header: &rtp::header::Header,
        encrypted: &[u8],
        roc: u32,
    ) -> Result<Vec<u8>, Error> {
        if encrypted.len() < self.auth_tag_len() {
            return Err(Error::new(format!(
                "too short SRTP packet: only {} bytes, expected > {} bytes",
                encrypted.len(),
                self.auth_tag_len()
            )));
        }

        let mut dst: Vec<u8> = Vec::with_capacity(encrypted.len() - self.auth_tag_len());

        // Split the auth tag and the cipher text into two parts.
        let actual_tag = &encrypted[encrypted.len() - self.auth_tag_len()..];
        let cipher_text = &encrypted[..encrypted.len() - self.auth_tag_len()];

        // Generate the auth tag we expect to see from the ciphertext.
        let expected_tag = self.generate_srtp_auth_tag(cipher_text, roc)?;

        // See if the auth tag actually matches.
        // We use a constant time comparison to prevent timing attacks.
        if actual_tag.ct_eq(&expected_tag).unwrap_u8() != 1 {
            return Err(Error::new("failed to verify auth tag".to_string()));
        }

        // Write cipher_text to the destination buffer.
        dst.extend_from_slice(cipher_text);

        // Decrypt the ciphertext for the payload.
        let counter = generate_counter(
            header.sequence_number,
            roc,
            header.ssrc,
            &self.srtp_session_salt,
        )?;

        let key = GenericArray::from_slice(&self.srtp_session_key);
        let nonce = GenericArray::from_slice(&counter);
        let mut stream = Aes128Ctr::new(&key, &nonce);

        stream.decrypt(&mut dst[header.payload_offset..]);

        Ok(dst)
    }

    fn encrypt_rtcp(
        &mut self,
        decrypted: &[u8],
        srtcp_index: u32,
        ssrc: u32,
    ) -> Result<Vec<u8>, Error> {
        let mut dst: Vec<u8> =
            Vec::with_capacity(decrypted.len() + SRTCP_INDEX_SIZE + self.auth_tag_len());

        // Write the decrypted to the destination buffer.
        dst.extend_from_slice(decrypted);

        // Encrypt everything after header
        let counter = generate_counter(
            (srtcp_index & 0xFFFF) as u16,
            srtcp_index >> 16,
            ssrc,
            &self.srtcp_session_salt,
        )?;

        let key = GenericArray::from_slice(&self.srtcp_session_key);
        let nonce = GenericArray::from_slice(&counter);
        let mut stream = Aes128Ctr::new(&key, &nonce);

        stream.encrypt(&mut dst[rtcp::header::HEADER_LENGTH + rtcp::header::SSRC_LENGTH..]);

        // Add SRTCP Index and set Encryption bit
        let mut srtcp_index_buffer: Vec<u8> = vec![];
        {
            let mut writer = BufWriter::new(&mut srtcp_index_buffer);
            writer.write_u32::<BigEndian>(srtcp_index | (1u32 << 31))?;
        }
        dst.extend_from_slice(&srtcp_index_buffer);

        // Generate the auth tag.
        let auth_tag = self.generate_srtcp_auth_tag(&dst)?;

        dst.extend_from_slice(&auth_tag);

        Ok(dst)
    }

    fn decrypt_rtcp(
        &mut self,
        encrypted: &[u8],
        srtcp_index: u32,
        ssrc: u32,
    ) -> Result<Vec<u8>, Error> {
        if encrypted.len() < self.auth_tag_len() + SRTCP_INDEX_SIZE {
            return Err(Error::new(format!(
                "too short SRTCP packet: only {} bytes, expected > {} bytes",
                encrypted.len(),
                self.auth_tag_len() + SRTCP_INDEX_SIZE,
            )));
        }

        let tail_offset = encrypted.len() - (self.auth_tag_len() + SRTCP_INDEX_SIZE);
        let mut dst: Vec<u8> = Vec::with_capacity(tail_offset);

        dst.extend_from_slice(&encrypted[0..tail_offset]);

        let is_encrypted = encrypted[tail_offset] >> 7;
        if is_encrypted == 0 {
            return Ok(dst);
        }

        // Split the auth tag and the cipher text into two parts.
        let actual_tag = &encrypted[encrypted.len() - self.auth_tag_len()..];
        let cipher_text = &encrypted[..encrypted.len() - self.auth_tag_len()];

        // Generate the auth tag we expect to see from the ciphertext.
        let expected_tag = self.generate_srtcp_auth_tag(cipher_text)?;

        // See if the auth tag actually matches.
        // We use a constant time comparison to prevent timing attacks.
        if actual_tag.ct_eq(&expected_tag).unwrap_u8() != 1 {
            return Err(Error::new("failed to verify auth tag".to_string()));
        }

        let counter = generate_counter(
            (srtcp_index & 0xFFFF) as u16,
            srtcp_index >> 16,
            ssrc,
            &self.srtcp_session_salt,
        )?;

        let key = GenericArray::from_slice(&self.srtcp_session_key);
        let nonce = GenericArray::from_slice(&counter);
        let mut stream = Aes128Ctr::new(&key, &nonce);

        stream.decrypt(&mut dst[rtcp::header::HEADER_LENGTH + rtcp::header::SSRC_LENGTH..]);

        Ok(dst)
    }
}
