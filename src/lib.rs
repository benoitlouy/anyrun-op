use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use fuzzy_matcher::FuzzyMatcher;
use serde::{Deserialize, Deserializer};
use std::io;
use std::process::Command;
use url::Url;

#[derive(Deserialize, Debug)]
struct OpListItem {
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

#[derive(Deserialize, Debug)]
struct OpGetItem {
    fields: Vec<OpField>,
}

#[derive(Deserialize, Debug)]
struct OpField {
    id: String,
    #[serde(alias = "type")]
    tpe: String,
    value: Option<String>,
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
    OpReturnCodeError(i32),
    ReadOutputError(std::string::FromUtf8Error),
    ParsingError(serde_json::Error),
}

struct State {
    items: Vec<(u64, OpListItem)>,
    selection: Option<Selection>,
}

struct Selection {
    id: String,
    username: Option<String>,
    password: Option<String>,
    has_otp: bool,
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
            Err(Error::OpReturnCodeError(o.status.code().unwrap()))
        }
    })
}

const ITEM_LIST_ARGS: [&str; 3] = ["item", "list", "--format=json"];
const DEFAULT_OP_CMD: &str = "op";

#[init]
fn init(config_dir: RString) -> State {
    let content = match execute_command(DEFAULT_OP_CMD, &ITEM_LIST_ARGS) {
        Err(Error::OpReturnCodeError(_)) => execute_command("op", &ITEM_LIST_ARGS),
        other => other,
    };

    let op_items = content
        .and_then(|s| {
            serde_json::from_str::<Vec<OpListItem>>(s.as_str()).map_err(Error::ParsingError)
        })
        .map(|items| {
            items
                .into_iter()
                .filter(|i| i.category == "PASSWORD" || i.category == "LOGIN")
                .enumerate()
                .map(|(id, item)| (id as u64, item))
                .collect::<Vec<_>>()
        });

    op_items
        .map(|items| State {
            items,
            selection: None,
        })
        .unwrap()
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
    match &state.selection {
        None => {
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

        Some(selection) => {
            let username = selection.username.as_ref().map(|_| Match {
                title: "Username".into(),
                icon: ROption::RNone,
                use_pango: false,
                description: ROption::RNone,
                id: ROption::RSome(0),
            });
            let password = selection.password.as_ref().map(|_| Match {
                title: "Password".into(),
                icon: ROption::RNone,
                use_pango: false,
                description: ROption::RNone,
                id: ROption::RSome(1),
            });
            let otp = if selection.has_otp {
                Some(Match {
                    title: "One-time password".into(),
                    icon: ROption::RNone,
                    use_pango: false,
                    description: ROption::RNone,
                    id: ROption::RSome(2),
                })
            } else {
                None
            };
            vec![username, password, otp]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .into()
        }
    }
}

#[handler]
fn handler(selection: Match, state: &mut State) -> HandleResult {
    match &state.selection {
        None => {
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

            let selected_item =
                execute_command("op", &["items", "get", id.as_str(), "--format=json"])
                    .and_then(|s| {
                        serde_json::from_str::<OpGetItem>(s.as_str()).map_err(Error::ParsingError)
                    })
                    .unwrap();

            let username = selected_item.fields.iter().find_map(|f| {
                if f.id == "username" {
                    f.value.clone()
                } else {
                    None
                }
            });

            let password = selected_item.fields.iter().find_map(|f| {
                if f.id == "password" {
                    f.value.clone()
                } else {
                    None
                }
            });

            let has_otp = selected_item.fields.iter().any(|f| f.tpe == "OTP");

            state.selection = Some(Selection {
                id,
                username,
                password,
                has_otp,
            });
            HandleResult::Refresh(true)
        }

        Some(s) => match selection.id {
            ROption::RSome(0) => HandleResult::Copy(s.username.as_ref().unwrap().as_bytes().into()),
            ROption::RSome(1) => HandleResult::Copy(s.password.as_ref().unwrap().as_bytes().into()),
            ROption::RSome(2) => execute_command("op", &["items", "get", s.id.as_str(), "--otp"])
                .map(|otp| HandleResult::Copy(otp.trim().as_bytes().into()))
                .unwrap(),
            _ => HandleResult::Close,
        },
    }
}
