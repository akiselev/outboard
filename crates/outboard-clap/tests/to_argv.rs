use std::{ffi::OsString,path::PathBuf};use clap::{ArgAction,Args,Parser,Subcommand,ValueEnum};use outboard_clap::{ToArgv,validate_args,validate_parser};
fn s(v:Vec<OsString>)->Vec<String>{v.into_iter().map(|x|x.into_string().unwrap()).collect()}
#[derive(Debug,Clone,Copy,PartialEq,Eq,ValueEnum)]enum Mode{Fast,Careful}
#[derive(Debug,Clone,PartialEq,Eq,Args,ToArgv)]struct Common{#[arg(long)]verbose:bool,#[arg(short='q',action=ArgAction::Count)]quiet:u8}
#[derive(Debug,Clone,PartialEq,Eq,Args,ToArgv)]struct RunArgs{input:PathBuf,#[arg(long)]output:Option<PathBuf>,#[arg(long="include")]includes:Vec<String>,#[arg(long,value_enum)]mode:Mode,#[arg(long,require_equals=true)]jobs:usize,#[command(flatten)]common:Common,#[arg(last=true)]trailing:Vec<OsString>}
#[derive(Debug,Clone,PartialEq,Eq,Subcommand,ToArgv)]enum Action{Run(RunArgs),Status,Named{#[arg(long)]label:String}}
#[derive(Debug,Clone,PartialEq,Eq,Parser,ToArgv)]struct Cli{#[command(subcommand)]action:Action}
#[test]fn parser_roundtrip(){let cli=Cli{action:Action::Run(RunArgs{input:"input.pdf".into(),output:Some("out.md".into()),includes:vec!["one".into(),"two".into()],mode:Mode::Careful,jobs:8,common:Common{verbose:true,quiet:2},trailing:vec!["--literal".into(),"value".into()]})};assert_eq!(s(validate_parser(&cli).unwrap()),vec!["run","input.pdf","--output","out.md","--include","one","--include","two","--mode","careful","--jobs=8","--verbose","-q","-q","--","--literal","value"])}
#[derive(Debug,Clone,PartialEq,Eq,Args,ToArgv)]struct Delimited{#[arg(long,value_delimiter=',')]values:Vec<String>,#[arg(long,action=ArgAction::SetFalse,default_value_t=true)]enabled:bool}
#[test]fn delimiter_and_set_false(){assert_eq!(s(validate_args(&Delimited{values:vec!["a".into(),"b".into()],enabled:false}).unwrap()),vec!["--values","a,b","--enabled"])}
#[derive(Debug,Clone,PartialEq,Eq,Args,ToArgv)]struct Constraint{#[arg(long,conflicts_with="beta")]alpha:Option<String>,#[arg(long)]beta:Option<String>}
#[test]fn clap_validates_constraints(){assert!(validate_args(&Constraint{alpha:Some("a".into()),beta:Some("b".into())}).is_err())}
#[test]fn enum_shapes(){assert_eq!(s(Cli{action:Action::Status}.to_argv()),vec!["status"]);assert_eq!(s(Cli{action:Action::Named{label:"x".into()}}.to_argv()),vec!["named","--label","x"])}
