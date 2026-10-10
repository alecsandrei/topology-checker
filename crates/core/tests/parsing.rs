use std::error::Error;
use topology_checker_core::{ParseRuleError, Rule};

#[test]
fn canonical_rules_round_trip_and_unknown_names_are_typed() {
    for &rule in Rule::ALL {
        assert_eq!(rule.name().parse::<Rule>().unwrap(), rule);
        assert_eq!(rule.to_string(), rule.name());
    }
    for name in [
        "",
        "Point/must-not-overlap",
        "point/must-not-overlap ",
        "unknown",
    ] {
        let error: ParseRuleError = name.parse::<Rule>().unwrap_err();
        assert_eq!(error.name(), name);
        assert_eq!(error.to_string(), format!("unknown rule {name}"));
        assert!(Error::source(&error).is_none());
    }
}
