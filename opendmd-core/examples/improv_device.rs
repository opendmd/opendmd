//! A pretend OpenDMD speaking Improv Serial on stdin/stdout, built on
//! `opendmd_core::improv` the way `firmware/src/improv.rs` uses it, so the
//! real host library (`improv-wifi-serial-sdk`, what ESP Web Tools runs)
//! can be pointed at it: `tools/improv-sdk-check.sh` (`make improv-check`).
//!
//! It answers like the firmware — junk first (what the host sees before the
//! console switches baud), log lines between frames, Ready until WiFi
//! details with the password `right` arrive, then Provisioned with a URL.

use opendmd_core::improv::{self as imp, Error, Event, Parser, Rpc, State};
use std::io::{Read, Write};

fn frame(len: usize, f: impl FnOnce(&mut [u8]) -> usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    let n = f(&mut v);
    v.truncate(n);
    v
}

fn state(s: State) -> Vec<u8> {
    frame(imp::STATE_FRAME, |o| imp::encode_state(s, o))
}

fn error(e: Error) -> Vec<u8> {
    frame(imp::STATE_FRAME, |o| imp::encode_error(e, o))
}

fn result(cmd: u8, strings: &[&str]) -> Vec<u8> {
    frame(imp::rpc_result_len(strings), |o| {
        imp::encode_rpc_result(cmd, strings, o)
    })
}

const URL: &str = "http://192.168.1.69/";

fn main() {
    let mut out = std::io::stdout().lock();
    let mut put = |b: &[u8]| {
        out.write_all(b).unwrap();
        out.flush().unwrap();
    };
    put(b"\xf8\x80\x00junk at the wrong baud, no newline");
    put(b"\n");
    put(b"I (100) opendmd_firmware: a boot line\n");
    let mut provisioned = false;
    let mut parser = Parser::new();
    let mut b = [0u8; 1];
    let mut input = std::io::stdin().lock();
    while input.read(&mut b).unwrap_or(0) == 1 {
        let Some(ev) = parser.push(b[0]) else {
            continue;
        };
        let send: Vec<Vec<u8>> = match ev {
            Event::BadChecksum => vec![error(Error::InvalidRpc)],
            Event::Frame {
                kind: imp::TYPE_RPC,
                data,
            } => match imp::parse_rpc(data) {
                Err(e) => vec![error(e)],
                Ok(Rpc::CurrentState) if provisioned => vec![
                    state(State::Provisioned),
                    result(imp::CMD_CURRENT_STATE, &[URL]),
                ],
                Ok(Rpc::CurrentState) => vec![state(State::Ready)],
                Ok(Rpc::DeviceInfo) => vec![result(
                    imp::CMD_DEVICE_INFO,
                    &["OpenDMD", "0.9.0", "ESP32", "opendmd"],
                )],
                Ok(Rpc::Scan) => {
                    let mut v = Vec::new();
                    // "a", "-1", "NO": exactly 10 bytes of data, padded.
                    for s in [
                        ["Home", "-48", "YES"],
                        ["Café Ñ", "-71", "NO"],
                        ["a", "-1", "NO"],
                    ] {
                        v.push(result(imp::CMD_SCAN, &s));
                        v.push(b"W (200) wifi: a log line between results\n".to_vec());
                    }
                    v.push(result(imp::CMD_SCAN, &[]));
                    v
                }
                Ok(Rpc::WifiSettings { ssid, password }) => {
                    eprintln!("device: WiFi details for {ssid:?}");
                    let mut v = vec![
                        error(Error::None),
                        state(State::Provisioning),
                        b"I (300) opendmd_firmware::net_worker: [net] improv: joining\n".to_vec(),
                    ];
                    if password == "right" {
                        provisioned = true;
                        v.push(state(State::Provisioned));
                        v.push(result(imp::CMD_WIFI_SETTINGS, &[URL]));
                    } else {
                        v.push(error(Error::UnableToConnect));
                        v.push(state(State::Ready));
                    }
                    v
                }
                Ok(Rpc::Other(_)) => vec![error(Error::UnknownRpc)],
            },
            Event::Frame { .. } => vec![],
        };
        for f in send {
            put(&f);
        }
    }
}
