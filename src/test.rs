use crate::*;

#[test]
fn test_config_extractors() -> Result<()> {
    let newcfg = Config::parse_from(vec!["x", "x", "k8s://test-context/test-namespace/test-pvc"]);
    assert_eq!("test-context", newcfg.get_remote_context()?);
    assert_eq!("test-namespace", newcfg.get_remote_namespace()?);
    assert_eq!("test-pvc", newcfg.get_remote_pvc()?);
    Ok(())
}

#[test]
fn test_config_extractors_relative() -> Result<()> {
    let newcfg = Config::parse_from(vec!["x", "x", "k8s:test-context/test-namespace/test-pvc"]);
    assert_eq!("test-context", newcfg.get_remote_context()?);
    assert_eq!("test-namespace", newcfg.get_remote_namespace()?);
    assert_eq!("test-pvc", newcfg.get_remote_pvc()?);
    Ok(())
}

#[test]
fn test_config_extractors_omitted_schema() -> Result<()> {
    let newcfg = Config::parse_from(vec!["x", "x", "test-context/test-namespace/test-pvc"]);
    assert_eq!("test-context", newcfg.get_remote_context()?);
    assert_eq!("test-namespace", newcfg.get_remote_namespace()?);
    assert_eq!("test-pvc", newcfg.get_remote_pvc()?);
    Ok(())
}

#[test]
fn test_config_extractors_omitted_schema_absolute_path() -> Result<()> {
    let newcfg = Config::parse_from(vec!["x", "x", "/test-context/test-namespace/test-pvc"]);
    assert_eq!("test-context", newcfg.get_remote_context()?);
    assert_eq!("test-namespace", newcfg.get_remote_namespace()?);
    assert_eq!("test-pvc", newcfg.get_remote_pvc()?);
    Ok(())
}

#[test]
fn test_config_extractors_trailing_slash() {
    let newcfg = Config::parse_from(vec!["x", "x", "k8s://test-context/test-namespace/test-pvc/"]);
    assert_eq!(
        newcfg.get_remote_pvc().unwrap_err().to_string(),
        ConfigError::RemoteTrailingElements.to_string(),
    );
}

#[test]
fn test_config_extractors_too_many_elements() {
    let newcfg = Config::parse_from(vec!["x", "x", "k8s://test-context/test-namespace/test-pvc/blah"]);
    assert_eq!(
        newcfg.get_remote_pvc().unwrap_err().to_string(),
        ConfigError::RemoteTrailingElements.to_string(),
    );
}

#[test]
fn test_config_extractors_blank_namespace() {
    let newcfg = Config::parse_from(vec!["x", "x", "k8s://test-context//test-pvc"]);
    let expected_err = newcfg.get_remote_namespace().expect_err("Expected RemoteNoNamespace error");
    assert_eq!(expected_err.to_string(), ConfigError::RemoteNoNamespace.to_string());
}

#[test]
fn test_config_extractors_blank_context() {
    let newcfg = Config::parse_from(vec!["x", "x", "k8s:///test-namespace/test-pvc"]);
    let expected_err = newcfg.get_remote_context().expect_err("Expected RemoteNoContext error");
    assert_eq!(expected_err.to_string(), ConfigError::RemoteNoContext.to_string());
}

#[test]
fn test_config_extractors_only_scheme() {
    let newcfg = Config::parse_from(vec!["x", "x", "k8s:"]);
    let expected_err = newcfg.get_remote_context().expect_err("Expected RemoteInvalid error");
    assert_eq!(expected_err.to_string(), ConfigError::RemoteInvalid.to_string());
}

#[test]
fn test_config_extractors_nothing() {
    let newcfg = Config::parse_from(vec!["x", "x", ""]);
    let expected_err = newcfg.get_remote_context().expect_err("Expected generic RemoteInvalid error");
    assert_eq!(expected_err.to_string(), ConfigError::RemoteInvalid.to_string());
}

#[test]
fn test_config_extractors_single_colon() {
    let newcfg = Config::parse_from(vec!["x", "x", ":"]);
    let expected_err = newcfg.get_remote_context().expect_err("Expected generic RemoteInvalid error");
    assert_eq!(expected_err.to_string(), ConfigError::RemoteInvalid.to_string());
}

#[test]
fn test_config_extractors_single_name() {
    let newcfg = Config::parse_from(vec!["x", "x", "ted"]);
    let expected_err = newcfg.get_remote_context().expect_err("Expected generic RemoteInvalid error");
    assert_eq!(expected_err.to_string(), ConfigError::RemoteInvalid.to_string());
}

#[test]
fn test_config_extractors_single_slash() {
    let newcfg = Config::parse_from(vec!["x", "x", "/"]);
    let expected_err = newcfg.get_remote_context().expect_err("Expected generic RemoteInvalid error");
    assert_eq!(expected_err.to_string(), ConfigError::RemoteInvalid.to_string());
}

#[test]
fn test_config_extractors_crazy_scheme() {
    let newcfg = Config::parse_from(vec!["x", "x", "crazyscheme://ctx/ns/pvc"]);
    let expected_err = newcfg.get_remote_context().expect_err("Expected generic RemoteInvalid error");
    assert_eq!(expected_err.to_string(), ConfigError::RemoteInvalidScheme.to_string());
}

#[test]
/// tests to ensure the appropriate error is returned in the face of many errors
/// specifically, if the scheme is invalid, anything else could be happening in
/// the url and it might not be an error _for that kind of URL_ 
/// so note first when the scheme is wrong because it might be the _only_ error
/// that's truly present.
fn test_config_extractors_crazy_scheme_and_other_problems() {
    let newcfg = Config::parse_from(vec!["x", "x", "crazyscheme:///ns"]);
    let expected_err = newcfg.get_remote_context().expect_err("Expected generic RemoteInvalid error");
    let eestr = expected_err.to_string();
    assert_eq!(eestr, ConfigError::RemoteInvalidScheme.to_string());
}
