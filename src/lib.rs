use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use fuzzy_matcher::FuzzyMatcher;
use serde::{Deserialize, Deserializer};
use std::io;
use std::process::Command;
use url::Url;

#[derive(Deserialize, Debug)]
struct OpItem {
    id: String,
    title: String,
    category: String,
    #[serde(default)]
    urls: Vec<OpUrl>,
}

#[derive(Deserialize, Debug)]
struct OpUrl {
    #[serde(deserialize_with = "host_from_url")]
    href: Option<String>,
}

fn host_from_url<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;

    Url::parse(&s)
        .map(|u| u.host_str().map(|s| s.to_string()))
        .or(Ok(Some(s)))
}

#[derive(Debug)]
enum Error {
    OpCommandFailed(io::Error),
    OpReturnCodeError { exit_code: i32 },
    ReadOutputError(std::string::FromUtf8Error),
    ParsingError(serde_json::Error),
}

struct State {
    items: Vec<(u64, OpItem)>,
}

fn execute_command(cmd: &str, args: &[&str]) -> Result<String, Error> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(Error::OpCommandFailed);

    output.and_then(|o| {
        if o.status.success() {
            String::from_utf8(o.stdout).map_err(Error::ReadOutputError)
        } else {
            Err(Error::OpReturnCodeError {
                exit_code: o.status.code().unwrap(),
            })
        }
    })
}

const ITEM_LIST_ARGS: [&str; 3] = ["item", "list", "--format=json"];
const DEFAULT_OP_CMD: &str = "op";

#[init]
fn init(config_dir: RString) -> State {
    let content = match execute_command(DEFAULT_OP_CMD, &ITEM_LIST_ARGS) {
        Err(Error::OpReturnCodeError { exit_code: _ }) => execute_command("op", &ITEM_LIST_ARGS),
        other => other,
    };

    let op_items = content
        .and_then(|s| serde_json::from_str::<Vec<OpItem>>(s.as_str()).map_err(Error::ParsingError))
        .map(|items| {
            items
                .into_iter()
                .filter(|i| i.category == "PASSWORD" || i.category == "LOGIN")
                .enumerate()
                .map(|(id, item)| (id as u64, item))
                .collect::<Vec<_>>()
        });

    op_items.map(|items| State { items }).unwrap()
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
    let matcher = fuzzy_matcher::skim::SkimMatcherV2::default().smart_case();

    let mut entries = state
        .items
        .iter()
        .filter_map(|(id, e)| {
            let title_score = matcher.fuzzy_match(&e.title, &input).unwrap_or(0);
            let domain_score = e
                .urls
                .iter()
                .flat_map(|u| u.href.clone())
                .map(|domain| matcher.fuzzy_match(&domain, &input).unwrap_or(0))
                .max()
                .unwrap_or(0);
            let score = std::cmp::max(title_score, domain_score);
            if score > 0 {
                Some((id, e, score))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| b.2.cmp(&a.2));
    entries.truncate(10);

    entries
        .into_iter()
        .map(|(id, e, _)| Match {
            title: e.title.clone().into(),
            icon: ROption::RNone,
            use_pango: false,
            description: ROption::RNone,
            id: ROption::RSome(*id as u64),
        })
        .collect()
}

#[handler]
fn handler(selection: Match, state: &State) -> HandleResult {
    let id = state
        .items
        .iter()
        .find_map(|(id, item)| {
            if *id == selection.id.unwrap() {
                Some(item.id.clone())
            } else {
                None
            }
        })
        .unwrap();

    execute_command("op", &["items", "get", id.as_str(), "--field", "password"])
        .map(|password| HandleResult::Copy(password.trim().as_bytes().into()))
        .unwrap()
}
