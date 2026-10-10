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

#[test]
fn original_registry_prefix_and_new_inputs_remain_stable() {
    let prefix = [
        "point/must-not-overlap",
        "point/must-not-overlap-with",
        "point/must-be-inside",
        "line/must-not-have-dangles",
        "line/must-not-have-pseudo-nodes",
        "line/must-not-overlap",
        "line/must-not-overlap-with",
        "line/must-not-self-overlap",
        "line/must-not-intersect",
        "line/must-not-intersect-with",
        "line/must-not-intersect-or-touch-interior",
        "line/must-not-intersect-or-touch-interior-with",
        "line/must-not-self-intersect",
        "line/must-be-inside",
        "polygon/must-not-overlap",
        "polygon/must-not-overlap-with",
        "polygon/must-not-have-gaps",
        "geometry/must-not-be-multipart",
    ];
    assert_eq!(Rule::ALL.len(), 38);
    assert_eq!(
        Rule::ALL[..18].iter().map(|r| r.name()).collect::<Vec<_>>(),
        prefix
    );
    assert!(Rule::ALL[18..].iter().all(|r| r.inputs().1.is_some()));
    assert_eq!(
        Rule::ALL
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        38
    );
}
