//! clap integration: inverse serialization (`T -> argv`), validation, schemas, and management CLI.
mod management;
mod schema;
pub use management::{
    OutputFormat, PluginCacheCommand, PluginCommand, PluginCommands, PluginDoctorArgs,
    PluginInspectArgs, PluginListArgs, PluginPathArgs, execute_plugin_command,
};
pub use outboard_clap_derive::ToArgv;
pub use schema::{
    CliArgSchema, CliCommandSchema, cli_schema, cli_schema_from_command, cli_schema_value,
};
use std::{
    ffi::{OsStr, OsString},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
};

pub trait ToArgv {
    fn append_argv(&self, argv: &mut Vec<OsString>);
    fn to_argv(&self) -> Vec<OsString> {
        let mut v = Vec::new();
        self.append_argv(&mut v);
        v
    }
}
pub trait ToArgValue {
    fn to_arg_value(&self) -> OsString;
}
impl<T: ToArgValue + ?Sized> ToArgValue for &T {
    fn to_arg_value(&self) -> OsString {
        (*self).to_arg_value()
    }
}
impl ToArgValue for String {
    fn to_arg_value(&self) -> OsString {
        self.into()
    }
}
impl ToArgValue for str {
    fn to_arg_value(&self) -> OsString {
        self.into()
    }
}
impl ToArgValue for OsString {
    fn to_arg_value(&self) -> OsString {
        self.clone()
    }
}
impl ToArgValue for OsStr {
    fn to_arg_value(&self) -> OsString {
        self.to_os_string()
    }
}
impl ToArgValue for PathBuf {
    fn to_arg_value(&self) -> OsString {
        self.as_os_str().to_os_string()
    }
}
impl ToArgValue for Path {
    fn to_arg_value(&self) -> OsString {
        self.as_os_str().to_os_string()
    }
}
impl ToArgValue for bool {
    fn to_arg_value(&self) -> OsString {
        self.to_string().into()
    }
}
impl ToArgValue for char {
    fn to_arg_value(&self) -> OsString {
        self.to_string().into()
    }
}
impl ToArgValue for IpAddr {
    fn to_arg_value(&self) -> OsString {
        self.to_string().into()
    }
}
impl ToArgValue for Ipv4Addr {
    fn to_arg_value(&self) -> OsString {
        self.to_string().into()
    }
}
impl ToArgValue for Ipv6Addr {
    fn to_arg_value(&self) -> OsString {
        self.to_string().into()
    }
}
impl ToArgValue for SocketAddr {
    fn to_arg_value(&self) -> OsString {
        self.to_string().into()
    }
}
macro_rules! display_values{($($t:ty),*$(,)?)=>{$(impl ToArgValue for $t{fn to_arg_value(&self)->OsString{self.to_string().into()}})*}}
display_values!(
    u8, u16, u32, u64, u128, usize, i8, i16, i32, i64, i128, isize, f32, f64
);
pub fn value_enum_to_os<T: clap::ValueEnum>(v: &T) -> OsString {
    v.to_possible_value()
        .map(|p| p.get_name().into())
        .unwrap_or_default()
}
#[doc(hidden)]
pub fn push_flag_value(argv: &mut Vec<OsString>, flag: &str, value: OsString, equals: bool) {
    if equals {
        let mut v = OsString::from(flag);
        v.push("=");
        v.push(value);
        argv.push(v)
    } else {
        argv.push(flag.into());
        argv.push(value)
    }
}
pub fn validate_args<T: ToArgv + clap::Args>(value: &T) -> Result<Vec<OsString>, clap::Error> {
    let argv = value.to_argv();
    T::augment_args(clap::Command::new("__outboard_validate")).try_get_matches_from(
        std::iter::once(OsString::from("__outboard_validate")).chain(argv.iter().cloned()),
    )?;
    Ok(argv)
}
pub fn validate_parser<T: ToArgv + clap::CommandFactory>(
    value: &T,
) -> Result<Vec<OsString>, clap::Error> {
    let argv = value.to_argv();
    T::command().try_get_matches_from(
        std::iter::once(OsString::from("__outboard_validate")).chain(argv.iter().cloned()),
    )?;
    Ok(argv)
}
pub trait ClapArgsToArgv: ToArgv + clap::Args {
    fn try_to_argv(&self) -> Result<Vec<OsString>, clap::Error> {
        validate_args(self)
    }
}
impl<T: ToArgv + clap::Args> ClapArgsToArgv for T {}
pub trait ClapParserToArgv: ToArgv + clap::CommandFactory {
    fn try_to_argv(&self) -> Result<Vec<OsString>, clap::Error> {
        validate_parser(self)
    }
}
impl<T: ToArgv + clap::CommandFactory> ClapParserToArgv for T {}
pub trait ResolvedPluginClapExt {
    fn command_typed<T: ToArgv>(&self, args: &T) -> std::process::Command;
}
impl ResolvedPluginClapExt for outboard::ResolvedPlugin {
    fn command_typed<T: ToArgv>(&self, args: &T) -> std::process::Command {
        self.command_with(args.to_argv())
    }
}
