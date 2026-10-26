#[derive(Debug, Clone, Default)]
pub struct Section {
    pub name: String,
    pub comment: String,
    pub keys: Vec<(String, String)>,
}

impl Section {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keys
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct Conf {
    pub sections: Vec<Section>,
}

impl Conf {
    pub fn parse(text: &str) -> Conf {
        let mut conf = Conf::default();
        let mut pending_comments: Vec<String> = Vec::new();
        let mut current: Option<Section> = None;

        for line in text.lines() {
            let trimmed = line.trim();

            if trimmed.is_empty() {
                pending_comments.clear();
                continue;
            }

            if trimmed.starts_with('#') || trimmed.starts_with(';') {
                pending_comments.push(trimmed.to_string());
                continue;
            }

            if let Some(header) = trimmed.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                if let Some(section) = current.take() {
                    conf.sections.push(section);
                }

                current = Some(Section {
                    name: header.trim().to_string(),
                    comment: pending_comments.join("\n"),
                    keys: Vec::new(),
                });
                pending_comments.clear();
                continue;
            }

            if let Some((key, value)) = trimmed.split_once('=') {
                if let Some(section) = current.as_mut() {
                    section
                        .keys
                        .push((key.trim().to_string(), value.trim().to_string()));
                }
            }
        }

        if let Some(section) = current.take() {
            conf.sections.push(section);
        }

        conf
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn sections_by_name<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Section> + 'a {
        self.sections
            .iter()
            .filter(move |s| s.name.eq_ignore_ascii_case(name))
    }

    pub fn interface(&self) -> Option<&Section> {
        self.section("Interface")
    }

    pub fn peers(&self) -> Vec<&Section> {
        self.sections_by_name("Peer").collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# my server
[Interface]
PrivateKey = aaa
Address = 10.0.0.2/32, fd00::2/128
DNS = 1.1.1.1

# first peer
[Peer]
PublicKey = bbb
AllowedIPs = 0.0.0.0/0
Endpoint = vpn.example.com:51820

# second peer
[Peer]
PublicKey = ccc
AllowedIPs = 10.1.0.0/16
";

    #[test]
    fn parses_duplicate_peer_sections() {
        let conf = Conf::parse(SAMPLE);
        assert_eq!(conf.peers().len(), 2);
        assert_eq!(conf.peers()[0].get("PublicKey"), Some("bbb"));
        assert_eq!(conf.peers()[1].get("PublicKey"), Some("ccc"));
    }

    #[test]
    fn parses_interface_and_comments() {
        let conf = Conf::parse(SAMPLE);
        let iface = conf.interface().unwrap();
        assert_eq!(iface.comment, "# my server");
        assert_eq!(iface.get("Address"), Some("10.0.0.2/32, fd00::2/128"));
        assert_eq!(conf.peers()[0].comment, "# first peer");
        assert_eq!(conf.peers()[1].comment, "# second peer");
    }

    #[test]
    fn key_lookup_is_case_insensitive() {
        let conf = Conf::parse(SAMPLE);
        assert_eq!(conf.peers()[0].get("presharedkey"), None);
        assert_eq!(conf.peers()[0].get("PUBLICKEY"), Some("bbb"));
    }
}
