//! `make ota-compat` (tools/ota-compat.sh) copies this into the
//! `opendmd-core/tests/` of an older release's checkout, and of this tree,
//! and runs it there: that tree's own verification code, as its firmware
//! runs it, against files signed now. The firmware's order
//! (firmware/src/ota.rs, www.rs): a bundle's header, its layout against the
//! player's (a 4mb player), the signature, that it fits the player's factory
//! partition, the body length, the streamed SHA-256, the app descriptor; a
//! www pack's header, its length against the body, every entry, the CRC.
//! Only API that 1.0.0-beta.14 and later all have.
//!
//! OTA_COMPAT_BUNDLE, OTA_COMPAT_PUB (32 raw bytes), OTA_COMPAT_WWW: paths
//! (each test skips when its file is not given). OTA_COMPAT_EXPECT: `take`
//! (every check passes) or `refuse` (refused at the header, before the
//! signature, so before anything is written). OTA_COMPAT_FACTORY: the
//! player's factory partition, bytes.

use opendmd_core::ota::{check_app_desc, verify_header, Header, ImageHasher, HEADER_LEN};
use opendmd_core::www_pack::{Header as WwwHeader, Pack};

fn path(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|p| !p.is_empty())
}

fn read(p: &str) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{p}: {e}"))
}

/// What the player makes of the bundle: `Ok(version, image bytes)`, or the
/// refusal and whether it came at the header (before the signature).
fn player_checks(data: &[u8], key: &[u8; 32], factory: u32) -> Result<(String, u32), (bool, String)> {
    let header = Header::parse(&data[..HEADER_LEN.min(data.len())]).map_err(|e| (true, e.to_string()))?;
    header.check_variant("4mb").map_err(|e| (true, e.to_string()))?;
    let meta = verify_header(&header, key).map_err(|e| (false, e.to_string()))?;
    meta.check_fits(factory).map_err(|e| (false, e.to_string()))?;
    let image = &data[HEADER_LEN..];
    if meta.length as usize != image.len() {
        return Err((false, format!("the body is not header + image ({} B, {} B)", meta.length, image.len())));
    }
    let mut h = ImageHasher::new();
    for piece in image.chunks(2048) {
        h.update(piece);
    }
    let (len, first, prefix) = (h.len(), h.first(), *h.prefix());
    meta.check_image(len, first, &h.finish()).map_err(|e| (false, e.to_string()))?;
    check_app_desc(&prefix, meta.variant).map_err(|e| (false, e.to_string()))?;
    Ok((meta.version, meta.length))
}

#[test]
fn the_player_takes_or_refuses_this_bundle() {
    let Some(bundle) = path("OTA_COMPAT_BUNDLE") else {
        return;
    };
    let data = read(&bundle);
    let key: [u8; 32] = read(&path("OTA_COMPAT_PUB").expect("OTA_COMPAT_PUB"))
        .try_into()
        .expect("a 32-byte public key");
    let factory: u32 = std::env::var("OTA_COMPAT_FACTORY")
        .expect("OTA_COMPAT_FACTORY")
        .parse()
        .expect("OTA_COMPAT_FACTORY: bytes");
    let expect = std::env::var("OTA_COMPAT_EXPECT").expect("OTA_COMPAT_EXPECT: take or refuse");
    let magic = String::from_utf8_lossy(&data[..8]).replace('\0', "\\0");
    match (player_checks(&data, &key, factory), expect.as_str()) {
        (Ok((version, length)), "take") => println!(
            "ota-compat: TAKEN as it should be: bundle {version} ({magic}, {length} B image) verifies with this \
             tree's code for a 4mb player with a {factory} B factory partition; its last 16 B: {:?}",
            String::from_utf8_lossy(&data[data.len() - 16..data.len() - 8])
        ),
        (Err((true, why)), "refuse") => println!(
            "ota-compat: REFUSED as it should be, at the header (before the signature, nothing written): \
             {magic} bundle ({} B image): {why}",
            data.len() - HEADER_LEN
        ),
        (Err((false, why)), "refuse") => {
            panic!("refused, but only after the header (it should not get that far): {why}")
        }
        (Ok((version, _)), _) => panic!("{magic} bundle {version} was taken, it should be refused"),
        (Err((_, why)), _) => panic!("{magic} bundle refused, it should be taken: {why}"),
    }
}

#[test]
fn the_player_takes_this_www_pack() {
    let Some(www) = path("OTA_COMPAT_WWW") else {
        return;
    };
    let data = read(&www);
    let header = WwwHeader::parse(&data).expect("the pack's header parses");
    assert_eq!(header.total_len as usize, data.len(), "the header's length is the body's");
    let pack = Pack::parse(&data).expect("every entry");
    pack.verify().expect("the CRC");
    assert!(pack.get("index.html").is_some(), "it has the app's index.html");
    println!(
        "ota-compat: www pack '{}' ({} B, {} entries) verifies with this tree's code",
        header.version(),
        data.len(),
        pack.count()
    );
}
