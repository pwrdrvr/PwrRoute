use super::*;
use crate::Catalog;
use proptest::prelude::*;

// Chromium process_singleton_posix.cc reads SplitString(message, "\\0",
// TRIM_WHITESPACE, SPLIT_WANT_ALL). Edge 156's committed sender capture
// establishes two additional fields before argv. Keep the reader independent
// of the encoder, including its whitespace alphabet and size limit.
const WS: &[u8] = b"\t\n\x0b\x0c\r ";
pub(crate) fn decode(bytes: &[u8], edge: bool) -> Option<Vec<String>> {
    if bytes.len() >= 32768 {
        return None;
    }
    let fields: Vec<_> = bytes
        .split(|b| *b == 0)
        .map(|s| {
            let start = s.iter().position(|b| !WS.contains(b)).unwrap_or(s.len());
            let end = s
                .iter()
                .rposition(|b| !WS.contains(b))
                .map_or(start, |i| i + 1);
            &s[start..end]
        })
        .collect();
    let start = if edge { 4 } else { 2 };
    if fields.len() <= start || fields[0] != b"START" {
        return None;
    }
    fields[start..]
        .iter()
        .map(|s| String::from_utf8(s.to_vec()).ok())
        .collect()
}

pub(crate) fn reader(id: &str) -> bool {
    match id {
        "chrome" | "chrome-beta" | "chrome-dev" | "chrome-canary" | "chromium" => false,
        "edge" | "edge-beta" | "edge-dev" | "edge-canary" => true,
        _ => panic!("{id} is socket eligible without a browser reader/capture oracle"),
    }
}

#[test]
fn every_socket_eligible_browser_has_an_independent_reader() {
    for entry in Catalog::builtin().browsers() {
        if let Some(protocol) = entry.singleton_protocol {
            assert_eq!(reader(&entry.id), protocol == SingletonProtocol::Edge);
        }
    }
    let chrome = include_bytes!("../../tests/fixtures/chrome-singleton-message.bin");
    let edge = include_bytes!("../../tests/fixtures/edge-156-singleton-message.bin");
    let c = decode(chrome, false).unwrap();
    let e = decode(edge, true).unwrap();
    assert!(c[0].ends_with("/Google Chrome"));
    assert_eq!(c.last().unwrap(), "https://example.com/chrome-capture");
    assert!(e[0].ends_with("/Microsoft Edge"));
    assert_eq!(e.last().unwrap(), "https://example.com/edge-capture");
}

fn check(url: String, args: Vec<String>) -> Result<(), TestCaseError> {
    for entry in Catalog::builtin().browsers() {
        let Some(protocol) = entry.singleton_protocol else {
            continue;
        };
        let edge = reader(&entry.id);
        let name = &entry.macos.as_ref().unwrap().app_name;
        let argv = crate::launch::properties::open_browser_args(name, url.clone(), args.clone());
        let h = header(protocol, Some(Duration::from_micros(123456))).unwrap();
        let size = 5 + 2 + h.iter().chain(&argv).map(|s| s.len() + 1).sum::<usize>();
        let bad = argv.iter().any(|s| {
            s.contains('\0')
                || s.as_bytes().first().is_some_and(|b| WS.contains(b))
                || s.as_bytes().last().is_some_and(|b| WS.contains(b))
        });
        match encode("/", &h, &argv) {
            Ok(bytes) => prop_assert_eq!(decode(&bytes, edge), Some(argv), "{}", entry.id),
            Err(SocketError::Unrepresentable) => prop_assert!(bad),
            Err(SocketError::TooLong) => prop_assert!(size >= 32768),
            Err(e) => return Err(TestCaseError::fail(format!("unjustified refusal: {e:?}"))),
        }
    }
    Ok(())
}

proptest! {
    #[test]
    fn each_browsers_reader_gets_exactly_the_open_argv(
        url in prop_oneof![any::<String>(), Just("x".repeat(32768)), Just("--a b;%'\"".into())],
        args in prop::collection::vec(any::<String>().prop_filter("open options use a separate transport", |s| s != "--args"), 0..4),
    ) { check(url, args)?; }
}

#[test]
fn edge_keeps_the_program_and_empty_argument_after_its_clock_fields() {
    check(String::new(), vec![String::new()]).unwrap();
}
