//! The few `Info.plist` keys detection needs, from XML property lists.
//! Browsers ship XML plists; a binary plist (`bplist00`) is reported as
//! unreadable and the caller falls back to the bundle's name.

use roxmltree::{Document, Node, ParsingOptions};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct BundleInfo {
    pub bundle_id: Option<String>,
    /// `CFBundleURLTypes` entries that list `http` or `https`, with their
    /// `LSHandlerRank` (absent means `Default`).
    pub web_handler_ranks: Vec<String>,
}

impl BundleInfo {
    /// Finicky's test: registers for http(s) with rank `Default` or
    /// `Alternate`. Apps ranked `None` claim the scheme for deep links or
    /// download interception and are not browsers.
    pub fn is_likely_browser(&self) -> bool {
        self.web_handler_ranks
            .iter()
            .any(|r| r == "Default" || r == "Alternate")
    }
}

pub(crate) fn parse(xml: &str) -> Option<BundleInfo> {
    // Every plist has a `<!DOCTYPE plist …>` naming Apple's external DTD,
    // which roxmltree never fetches; without allow_dtd it refuses the file.
    let options = ParsingOptions {
        allow_dtd: true,
        ..ParsingOptions::default()
    };
    let doc = Document::parse_with_options(xml, options).ok()?;
    let dict = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name("dict"))?;
    let mut info = BundleInfo {
        bundle_id: lookup(dict, "CFBundleIdentifier")
            .and_then(string)
            .map(str::to_owned),
        ..Default::default()
    };
    if let Some(types) = lookup(dict, "CFBundleURLTypes") {
        for url_type in types.children().filter(|n| n.has_tag_name("dict")) {
            let web = lookup(url_type, "CFBundleURLSchemes").is_some_and(|schemes| {
                schemes
                    .children()
                    .filter_map(string)
                    .any(|s| s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https"))
            });
            if web {
                let rank = lookup(url_type, "LSHandlerRank").and_then(string);
                info.web_handler_ranks
                    .push(rank.unwrap_or("Default").to_owned());
            }
        }
    }
    Some(info)
}

/// The value element after `<key>name</key>` in a `<dict>`.
fn lookup<'a, 'i>(dict: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    let mut elements = dict.children().filter(Node::is_element);
    while let Some(key) = elements.next() {
        let value = elements.next()?;
        if key.has_tag_name("key") && key.text() == Some(name) {
            return Some(value);
        }
    }
    None
}

fn string<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    node.has_tag_name("string")
        .then(|| node.text().unwrap_or_default().trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bundle_id_and_handlers() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Example</string>
  <key>CFBundleIdentifier</key><string>com.example.browser</string>
  <key>CFBundleURLTypes</key><array>
    <dict><key>CFBundleURLSchemes</key><array><string>mailto</string></array></dict>
    <dict><key>CFBundleURLName</key><string>Web</string>
      <key>CFBundleURLSchemes</key><array><string>http</string><string>https</string></array></dict>
  </array>
</dict></plist>"#;
        let info = parse(xml).unwrap();
        assert_eq!(info.bundle_id.as_deref(), Some("com.example.browser"));
        assert_eq!(info.web_handler_ranks, ["Default"]);
        assert!(info.is_likely_browser());
    }

    #[test]
    fn rank_none_is_not_a_browser() {
        let xml = r#"<plist><dict><key>CFBundleURLTypes</key><array><dict>
            <key>LSHandlerRank</key><string>None</string>
            <key>CFBundleURLSchemes</key><array><string>https</string></array>
            </dict></array></dict></plist>"#;
        assert!(!parse(xml).unwrap().is_likely_browser());
        assert_eq!(parse("bplist00\u{0}"), None);
    }
}
