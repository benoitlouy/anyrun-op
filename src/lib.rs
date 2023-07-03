use abi_stable::std_types::{ROption, RString, RVec};
use anyrun_plugin::*;
use fuzzy_matcher::FuzzyMatcher;
use serde::{Deserialize, Deserializer};
use std::fs;
use std::io;
use std::process::Command;
use url::Url;

#[derive(Deserialize)]
struct Config {
    #[serde(default = "max_entries")]
    max_entries: usize,
    #[serde(default = "op_path")]
    op_path: String,
    #[serde(default = "prefix")]
    prefix: String,
}

fn max_entries() -> usize {
    10
}

fn op_path() -> String {
    "op".into()
}

fn prefix() -> String {
    "".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_entries: max_entries(),
            op_path: op_path(),
            prefix: prefix(),
        }
    }
}

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
    config: Config,
    items: Vec<(u64, OpListItem)>,
    input: Option<String>,
    selection: Option<Selection>,
}

struct Selection {
    id: String,
    username: Option<String>,
    password: Option<String>,
    has_otp: bool,
    ccnum: Option<String>,
    cvv: Option<String>,
    expiry: Option<String>,
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

#[init]
fn init(config_dir: RString) -> State {
    let config: Config = load_config(config_dir);

    let content = match execute_command(&config.op_path, &ITEM_LIST_ARGS) {
        Err(Error::OpReturnCodeError(_)) => execute_command(&config.op_path, &ITEM_LIST_ARGS),
        other => other,
    };

    let op_items = content
        .and_then(|s| {
            serde_json::from_str::<Vec<OpListItem>>(s.as_str()).map_err(Error::ParsingError)
        })
        .map(|items| {
            items
                .into_iter()
                .filter(|i| i.category == "PASSWORD" || i.category == "LOGIN" || i.category == "CREDIT_CARD")
                .enumerate()
                .map(|(id, item)| (id as u64, item))
                .collect::<Vec<_>>()
        });

    op_items
        .map(|items| State {
            config,
            items,
            input: None,
            selection: None,
        })
        .unwrap()
}

#[info]
fn info() -> PluginInfo {
    PluginInfo {
        name: "1password".into(),
        icon: "1password".into(), // Icon from the icon theme
    }
}

fn load_config(config_dir: RString) -> Config {
    match fs::read_to_string(format!("{}/op.ron", config_dir)) {
        Ok(content) => ron::from_str(&content).unwrap_or_else(|why| {
            eprintln!("Error parsing op plugin config: {}", why);
            Config::default()
        }),
        Err(why) => {
            eprintln!("Error reading op plugin config: {}", why);
            Config::default()
        }
    }
}

#[get_matches]
fn get_matches(input: RString, state: &mut State) -> RVec<Match> {
    match &state.selection {
        None => display_matching_items(&input, state),
        Some(selection) => match &state.input {
            None => display_matching_items(&input, state),
            Some(s) => {
                if input.as_str() == s {
                    display_selection_items(selection)
                } else {
                    state.selection = None;
                    state.input = None;
                    display_matching_items(&input, state)
                }
            }
        },
    }
}

fn display_matching_items(input: &RString, state: &mut State) -> RVec<Match> {
    if !input.starts_with(&state.config.prefix) {
        return RVec::new();
    }

    let cleaned_input = &input[state.config.prefix.len()..];
    let matcher = fuzzy_matcher::skim::SkimMatcherV2::default().smart_case();

    let mut entries = state
        .items
        .iter()
        .filter_map(|(id, e)| {
            let title_score = matcher.fuzzy_match(&e.title, cleaned_input).unwrap_or(0);
            let domain_score = e
                .urls
                .iter()
                .flat_map(|u| u.href.clone())
                .map(|domain| matcher.fuzzy_match(&domain, cleaned_input).unwrap_or(0))
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
    entries.truncate(state.config.max_entries);

    state.input = Some(input.to_string());

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

fn display_selection_items(selection: &Selection) -> RVec<Match> {
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

    let ccnum = selection.ccnum.as_ref().map(|_| Match {
        title: "Number".into(),
        icon: ROption::RNone,
        use_pango: false,
        description: ROption::RNone,
        id: ROption::RSome(3),
    });

    let ccv = selection.cvv.as_ref().map(|_| Match {
        title: "CCV".into(),
        icon: ROption::RNone,
        use_pango: false,
        description: ROption::RNone,
        id: ROption::RSome(4),
    });

    let expiry = selection.expiry.as_ref().map(|_| Match {
        title: "Expiry".into(),
        icon: ROption::RNone,
        use_pango: false,
        description: ROption::RNone,
        id: ROption::RSome(5),
    });

    vec![username, password, otp, ccnum, ccv, expiry]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .into()
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

            let selected_item = execute_command(
                &state.config.op_path,
                &["items", "get", id.as_str(), "--format=json"],
            )
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

            let ccnum = selected_item.fields.iter().find_map(|f| {
                if f.id == "ccnum" {
                    f.value.clone()
                } else {
                    None
                }
            });

            let cvv = selected_item.fields.iter().find_map(|f| {
                if f.id == "cvv" {
                    f.value.clone()
                } else {
                    None
                }
            });

            let expiry = selected_item.fields.iter().find_map(|f| {
                if f.id == "expiry" {
                    f.value.clone()
                } else {
                    None
                }
            });

            state.selection = Some(Selection {
                id,
                username,
                password,
                has_otp,
                ccnum,
                cvv,
                expiry,
            });

            HandleResult::Refresh(true)
        }

        Some(s) => match selection.id {
            ROption::RSome(0) => HandleResult::Copy(s.username.as_ref().unwrap().as_bytes().into()),
            ROption::RSome(1) => HandleResult::Copy(s.password.as_ref().unwrap().as_bytes().into()),
            ROption::RSome(2) => execute_command(
                &state.config.op_path,
                &["items", "get", s.id.as_str(), "--otp"],
            )
            .map(|otp| HandleResult::Copy(otp.trim().as_bytes().into()))
            .unwrap(),
            ROption::RSome(3) => HandleResult::Copy(s.ccnum.as_ref().unwrap().as_bytes().into()),
            ROption::RSome(4) => HandleResult::Copy(s.cvv.as_ref().unwrap().as_bytes().into()),
            ROption::RSome(5) => HandleResult::Copy(s.expiry.as_ref().unwrap().as_bytes().into()),
            _ => HandleResult::Close,
        },
    }
}
