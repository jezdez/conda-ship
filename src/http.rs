pub(crate) const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::USER_AGENT;

    #[test]
    fn user_agent_names_conda_ship() {
        assert!(USER_AGENT.starts_with("conda-ship/"));
    }
}
