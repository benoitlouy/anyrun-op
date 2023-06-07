use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use serde::Deserialize;
use std::io::{self, stdin, Write};
use std::process::Command;

#[derive(Deserialize, Debug)]
struct OpItem {
    id: String,
    title: String,
    category: String,
}

#[derive(Debug)]
enum Error {
    OpCommandFailed(io::Error),
    ReadOutputError(std::string::FromUtf8Error),
    ParsingError(serde_json::Error),
}

struct State {
    items: Vec<OpItem>,
}

#[init]
fn init(config_dir: RString) -> State {
    let output = Command::new("op")
        .args(["item", "list", "--format=json"])
        .output()
        .map_err(Error::OpCommandFailed);

    let content = output.and_then(|o| String::from_utf8(o.stdout).map_err(Error::ReadOutputError));

    let op_items = content
        .and_then(|s| serde_json::from_str::<Vec<OpItem>>(s.as_str()).map_err(Error::ParsingError));

    println!("{:?}", op_items);

    op_items.map(|items| State{items}).unwrap()
}

#[info]
fn info() -> PluginInfo {
    PluginInfo {
        name: "op".into(),
        icon: "help-about".into(), // Icon from the icon theme
    }
}

#[get_matches]
fn get_matches(input: RString, state: &State) -> RVec<Match> {
    // The logic to get matches from the input text in the `input` argument.
    // The `data` is a mutable reference to the shared data type later specified.
    vec![Match {
        title: "Test match".into(),
        icon: ROption::RSome("help-about".into()),
        use_pango: false,
        description: ROption::RSome("Test match for the plugin API demo".into()),
        id: ROption::RNone, // The ID can be used for identifying the match la =ter, is not required
    }]
    .into()
}

#[handler]
fn handler(selection: Match, state: &State) -> HandleResult {
    // Handle the selected match and return how anyrun should proceed
    HandleResult::Close
}
