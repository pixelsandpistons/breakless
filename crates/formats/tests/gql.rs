//! The GraphQL requests the service sends and the reading of their answers, against what the
//! prototype's `twitchlib.py` does with the same canned answers (its output is quoted here).

use breakless_formats::gql;
use serde_json::{Value, json};

#[test]
fn a_token_request_carries_the_query_and_its_three_variables() {
    let r = gql::token_request("Alpha", "site", "web");
    assert_eq!(r["query"], gql::TOKEN_QUERY);
    assert_eq!(
        r["variables"],
        json!({"login": "Alpha", "playerType": "site", "platform": "web"})
    );
}

#[test]
fn a_live_request_lowercases_names_drops_invalid_ones_and_is_not_made_for_none() {
    let logins: Vec<String> = ["Alpha", "beta", "GAMMA", "delta", "eps", "bad name!", "x"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let r = gql::live_request(&logins).unwrap();
    assert_eq!(r["query"], gql::LIVE_QUERY);
    assert_eq!(
        r["variables"],
        json!({"logins": ["alpha", "beta", "gamma", "delta", "eps"]})
    );
    assert_eq!(gql::live_request(&["bad name!".to_string()]), None);
    assert_eq!(gql::live_request(&[]), None);
    let t = gql::top_request(3);
    assert_eq!(t["query"], gql::TOP_QUERY);
    assert_eq!(t["variables"], json!({"n": 3}));
}

#[test]
fn an_answer_gives_its_data_or_the_fault_in_one_line() {
    assert_eq!(gql::data(r#"{"data": {"a": 1}}"#).unwrap(), json!({"a": 1}));
    assert_eq!(
        gql::data(r#"{"errors": [], "data": null}"#).unwrap(),
        json!({})
    );
    assert_eq!(
        gql::data(r#"{"errors": [{"message": "no"}], "data": null}"#).unwrap_err(),
        r#"GQL: [{"message":"no"}]"#
    );
    assert!(
        gql::data("<html>")
            .unwrap_err()
            .starts_with("GQL: not JSON: ")
    );
    let long = format!(r#"{{"errors": [{{"message": "{}"}}]}}"#, "x".repeat(400));
    assert_eq!(
        gql::data(&long).unwrap_err().chars().count(),
        "GQL: ".len() + 300
    );
}

#[test]
fn a_token_needs_both_halves() {
    let data = json!({"streamPlaybackAccessToken": {"value": "{\"a\":1}", "signature": "0ab"}});
    let t = gql::token(&data).unwrap();
    assert_eq!(
        (t.value.as_str(), t.signature.as_str()),
        ("{\"a\":1}", "0ab")
    );
    assert_eq!(
        gql::token(&json!({"streamPlaybackAccessToken": null})),
        None
    );
    assert_eq!(
        gql::token(&json!({"streamPlaybackAccessToken": {"value": "x"}})),
        None
    );
    assert_eq!(gql::token(&json!({})), None);
}

#[test]
fn live_channels_are_read_as_the_prototype_reads_them() {
    let data = json!({"users": [
      {"login": "alpha", "displayName": "Alpha", "stream": {"title": "Speedrun", "viewersCount": 1234, "game": {"displayName": "Games"}}},
      {"login": "beta", "displayName": "Beta", "stream": null},
      {"login": "gamma", "displayName": "Gamma", "stream": {}},
      {"login": "delta", "displayName": "Delta", "stream": {"title": null, "viewersCount": null, "game": null}},
      null,
      {"login": "eps", "displayName": "Eps", "stream": {"viewersCount": 7}},
    ]});
    let got = serde_json::to_value(gql::live(&data)).unwrap();
    let python: Value = serde_json::from_str(r#"{"alpha": {"game": "Games", "name": "Alpha", "title": "Speedrun", "viewers": 1234}, "delta": {"game": "", "name": "Delta", "title": "", "viewers": 0}, "eps": {"game": "", "name": "Eps", "title": "", "viewers": 7}}"#).unwrap();
    assert_eq!(got, python);
}

#[test]
fn top_channels_are_read_as_the_prototype_reads_them() {
    let data = json!({"streams": {"edges": [
      {"node": {"title": "One", "viewersCount": 90000, "broadcaster": {"login": "one", "displayName": "One!"}, "game": {"displayName": "Chatting"}}},
      {"node": {"title": "No one", "viewersCount": 5, "broadcaster": null, "game": null}},
      {"node": {"title": null, "viewersCount": null, "broadcaster": {"login": "three", "displayName": "Three"}, "game": null}},
    ]}});
    let got = serde_json::to_value(gql::top(&data)).unwrap();
    let python: Value = serde_json::from_str(r#"[{"game": "Chatting", "login": "one", "name": "One!", "title": "One", "viewers": 90000}, {"game": "", "login": "three", "name": "Three", "title": "", "viewers": 0}]"#).unwrap();
    assert_eq!(got, python);
    assert_eq!(gql::top(&json!({"streams": null})), Vec::<gql::Top>::new());
}
