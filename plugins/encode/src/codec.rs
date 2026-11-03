use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use sha2::Digest;

pub const ENCODE_FORMATS: [&str; 6] = ["base64", "md5", "url", "sha1", "sha256", "sha512"];
pub const DECODE_FORMATS: [&str; 2] = ["base64", "url"];
const URI_COMPONENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn transform(format: &str, input: &str, decode: bool) -> Result<String, ()> {
    if !decode {
        return Ok(match format {
            "base64" => STANDARD.encode(input),
            "url" => utf8_percent_encode(input, URI_COMPONENT).to_string(),
            "md5" => hex(&md5::Md5::digest(input)),
            "sha1" => hex(&sha1::Sha1::digest(input)),
            "sha256" => hex(&sha2::Sha256::digest(input)),
            "sha512" => hex(&sha2::Sha512::digest(input)),
            _ => return Err(()),
        });
    }
    match format {
        "base64" => {
            let value: String = input.chars().filter(|ch| !ch.is_whitespace()).collect();
            let bytes = STANDARD
                .decode(&value)
                .or_else(|_| STANDARD_NO_PAD.decode(&value))
                .map_err(|_| ())?;
            String::from_utf8(bytes).map_err(|_| ())
        }
        "url" => {
            let mut bytes = Vec::new();
            let mut source = input.as_bytes().iter().copied();
            while let Some(byte) = source.next() {
                if byte == b'%' {
                    let high = (source.next().ok_or(())? as char).to_digit(16).ok_or(())?;
                    let low = (source.next().ok_or(())? as char).to_digit(16).ok_or(())?;
                    bytes.push((high * 16 + low) as u8);
                } else {
                    bytes.push(byte);
                }
            }
            String::from_utf8(bytes).map_err(|_| ())
        }
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_round_trip_for_every_reversible_format() {
        let input = " 中文 &<>\"'\n+/%🙂 ";
        for format in DECODE_FORMATS {
            assert_eq!(
                transform(format, &transform(format, input, false).unwrap(), true).unwrap(),
                input,
                "{format}"
            );
        }
    }
    #[test]
    fn rejects_malformed_and_binary_decoding() {
        for (format, input) in [
            ("base64", "A==="),
            ("base64", "/w=="),
            ("url", "%GG"),
            ("url", "%ff"),
        ] {
            assert!(transform(format, input, true).is_err(), "{format}: {input}");
        }
        assert_eq!(transform("url", "a+b", true).unwrap(), "a+b");
        assert_eq!(
            transform("sha256", "abc", false).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
