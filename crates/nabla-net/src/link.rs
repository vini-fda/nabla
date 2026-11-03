//! What every platform's glue has in common: where a server is, how to trust it, and what comes
//! back from the connection.
//!
//! The glue itself differs: `tokio` and `wtransport` natively ([`native`](crate::native)), and
//! `web_sys::WebTransport` in the browser. Each turns a WebTransport session into the same
//! thing: [`Kind`](crate::Kind)s going out, [`Incoming`]s coming in.

use std::fmt::Write as _;

/// Something the connection delivered to the machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incoming {
    /// One whole frame from the stream.
    Stream(Vec<u8>),
    /// One datagram.
    Datagram(Vec<u8>),
    /// The connection is over, and why.
    Closed(String),
}

/// A server's address, and how to know it is the server.
///
/// It is written `https://host:port`, with `#sha256=` and the 64 hex digits of its certificate's
/// hash after it, if the certificate is self-signed. The hash is in the fragment because a page
/// cannot learn it over the connection it authenticates, and the fragment is never sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub url: String,
    /// The `SHA-256` of the server's certificate, in place of a chain to a trusted root.
    pub hash: Option<[u8; 32]>,
}

impl Address {
    /// # Errors
    /// If the text is not an `https` address, or its hash is not 64 hex digits.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (url, fragment) = match text.split_once('#') {
            Some((url, fragment)) => (url, Some(fragment)),
            None => (text, None),
        };
        if !url.starts_with("https://") || url.len() == "https://".len() {
            return Err("a server's address begins with https://".to_owned());
        }
        let hash = fragment
            .map(|fragment| {
                let digits = fragment
                    .strip_prefix("sha256=")
                    .ok_or("after the # comes sha256= and the certificate's hash")?;
                parse_hash(digits)
            })
            .transpose()?;
        Ok(Self {
            url: url.to_owned(),
            hash,
        })
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.url)?;
        if let Some(hash) = &self.hash {
            write!(f, "#sha256={}", hex(hash))?;
        }
        Ok(())
    }
}

/// The 64 hex digits of a `SHA-256` hash.
///
/// # Errors
/// If `text` is not exactly 64 hex digits.
pub fn parse_hash(text: &str) -> Result<[u8; 32], String> {
    let bytes = text.as_bytes();
    if bytes.len() != 64 {
        return Err("a certificate hash is 64 hex digits".to_owned());
    }
    let mut out = [0_u8; 32];
    let (pairs, _) = bytes.as_chunks::<2>();
    for (byte, pair) in out.iter_mut().zip(pairs) {
        let digit = |c: u8| char::from(c).to_digit(16);
        let (Some(high), Some(low)) = (digit(pair[0]), digit(pair[1])) else {
            return Err("a certificate hash is 64 hex digits".to_owned());
        };
        *byte = u8::try_from(high * 16 + low).expect("two hex digits fit a byte");
    }
    Ok(out)
}

/// `bytes` as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        write!(out, "{byte:02x}").ok();
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_round_trips() {
        let hash: [u8; 32] = std::array::from_fn(|i| u8::try_from(i * 7).expect("small"));
        let address = Address {
            url: "https://localhost:4433".to_owned(),
            hash: Some(hash),
        };
        assert_eq!(Address::parse(&address.to_string()), Ok(address));
        assert_eq!(
            Address::parse("https://example.com"),
            Ok(Address {
                url: "https://example.com".to_owned(),
                hash: None
            })
        );
    }

    #[test]
    fn bad_addresses_are_refused() {
        for bad in [
            "",
            "http://x",
            "https://",
            "https://x#",
            "https://x#sha256=12",
            "https://x#hash=00",
            &format!("https://x#sha256={}", "zz".repeat(32)),
        ] {
            assert!(Address::parse(bad).is_err(), "{bad:?}");
        }
    }
}
