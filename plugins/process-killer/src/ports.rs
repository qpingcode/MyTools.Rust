use std::{collections::HashSet, process::Command};

pub fn query_port(query: &str) -> Result<Option<u16>, ()> {
    let query = query.trim();
    let explicit = query
        .strip_prefix(':')
        .or_else(|| query.strip_prefix("port "));
    let value = explicit.unwrap_or(query);
    if explicit.is_some() || (!value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit())) {
        let port = value.trim().parse::<u16>().map_err(|_| ())?;
        if port == 0 {
            return Err(());
        }
        Ok(Some(port))
    } else {
        Ok(None)
    }
}

pub fn owners(port: u16) -> Result<HashSet<u32>, String> {
    #[cfg(windows)]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let mut command = Command::new("netstat.exe");
        command.args(["-ano"]).creation_flags(0x08000000);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("lsof");
        command.args(["-nP", &format!("-i:{port}"), "-Fp"]);
        command
    };
    let output = command.output().map_err(|e| e.to_string())?;
    #[cfg(not(windows))]
    if output.status.code() == Some(1) && output.stdout.is_empty() {
        return Ok(HashSet::new());
    }
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into());
    }
    Ok(parse(&String::from_utf8_lossy(&output.stdout), port))
}

fn parse(output: &str, port: u16) -> HashSet<u32> {
    #[cfg(windows)]
    return output
        .lines()
        .filter_map(|line| {
            let columns: Vec<_> = line.split_whitespace().collect();
            if !matches!(columns.first(), Some(&"TCP" | &"UDP")) {
                return None;
            }
            let (_, local_port) = columns.get(1)?.rsplit_once(':')?;
            (local_port.parse::<u16>().ok()? == port)
                .then(|| columns.last()?.parse().ok())
                .flatten()
        })
        .collect();
    #[cfg(not(windows))]
    {
        let _ = port;
        output
            .lines()
            .filter_map(|line| line.strip_prefix('p')?.parse().ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_treats_valid_exact_ports_as_port_queries() {
        assert_eq!(query_port(":8080"), Ok(Some(8080)));
        assert_eq!(query_port("port 443"), Ok(Some(443)));
        assert_eq!(query_port("chrome"), Ok(None));
        assert!(query_port("65536").is_err());
        assert!(query_port(":0").is_err());
    }
    #[cfg(windows)]
    #[test]
    fn matches_local_ipv4_ipv6_and_udp_but_not_remote_or_partial_ports() {
        let output = "TCP 127.0.0.1:8080 0.0.0.0:0 LISTENING 42\nTCP [::]:8080 [::]:0 LISTENING 43\nUDP 0.0.0.0:8080 *:* 44\nTCP 127.0.0.1:18080 1.2.3.4:8080 ESTABLISHED 45";
        assert_eq!(parse(output, 8080), HashSet::from([42, 43, 44]));
    }
}
