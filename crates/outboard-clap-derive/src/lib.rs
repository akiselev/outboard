use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Attribute, Data, DataEnum, DataStruct, DeriveInput, Expr, Fields, GenericArgument, Lit, Meta,
    PathArguments, Token, Type, parse_macro_input, punctuated::Punctuated,
};

#[proc_macro_derive(ToArgv, attributes(arg, command, outboard))]
pub fn derive_to_argv(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn runtime_path() -> TokenStream2 {
    match crate_name("outboard-clap") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name.replace('-', "_"));
            quote!(::#ident)
        }
        Err(_) => quote!(::outboard_clap),
    }
}

fn expand(input: DeriveInput) -> syn::Result<TokenStream2> {
    let runtime = runtime_path();
    let name = input.ident;
    let generics = input.generics;
    let rename_all = container_rename_all(&input.attrs).unwrap_or_else(|| "kebab-case".to_owned());
    let body = match input.data {
        Data::Struct(data) => expand_struct(&data, &rename_all, &runtime)?,
        Data::Enum(data) => expand_enum(&data, &rename_all, &runtime)?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &name,
                "ToArgv cannot be derived for unions",
            ));
        }
    };
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics #runtime::ToArgv for #name #ty_generics #where_clause {
            fn append_argv(&self, __outboard_argv: &mut ::std::vec::Vec<::std::ffi::OsString>) { #body }
        }
    })
}

fn expand_struct(
    data: &DataStruct,
    rename_all: &str,
    runtime: &TokenStream2,
) -> syn::Result<TokenStream2> {
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "ToArgv requires named struct fields",
        ));
    };
    let mut out = TokenStream2::new();
    for field in &fields.named {
        let ident = field.ident.as_ref().expect("named");
        let cfg = FieldCfg::parse(&field.attrs, &ident.to_string(), rename_all)?;
        out.extend(emit_field(quote!(&self.#ident), &field.ty, &cfg, runtime)?);
    }
    Ok(out)
}

fn expand_enum(
    data: &DataEnum,
    rename_all: &str,
    runtime: &TokenStream2,
) -> syn::Result<TokenStream2> {
    let mut arms = Vec::new();
    for variant in &data.variants {
        let ident = &variant.ident;
        let command_name = variant_name(&variant.attrs, &ident.to_string(), rename_all)?;
        match &variant.fields {
            Fields::Unit => arms.push(quote! { Self::#ident => __outboard_argv.push(::std::ffi::OsString::from(#command_name)) }),
            Fields::Unnamed(fields) => {
                if fields.unnamed.len() != 1 { return Err(syn::Error::new_spanned(fields, "tuple subcommands require exactly one payload")); }
                arms.push(quote! { Self::#ident(__outboard_inner) => { __outboard_argv.push(::std::ffi::OsString::from(#command_name)); #runtime::ToArgv::append_argv(__outboard_inner, __outboard_argv); } });
            }
            Fields::Named(fields) => {
                let bindings: Vec<_> = fields.named.iter().map(|f| f.ident.clone().expect("named")).collect();
                let rename = container_rename_all(&variant.attrs).unwrap_or_else(|| rename_all.to_owned());
                let mut emitted = TokenStream2::new();
                for field in &fields.named {
                    let field_ident = field.ident.as_ref().expect("named");
                    let cfg = FieldCfg::parse(&field.attrs, &field_ident.to_string(), &rename)?;
                    emitted.extend(emit_field(quote!(#field_ident), &field.ty, &cfg, runtime)?);
                }
                arms.push(quote! { Self::#ident { #(#bindings),* } => { __outboard_argv.push(::std::ffi::OsString::from(#command_name)); #emitted } });
            }
        }
    }
    Ok(quote! { match self { #(#arms),* } })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Action {
    #[default]
    Default,
    Value,
    SetTrue,
    SetFalse,
    Count,
}
#[derive(Debug, Default)]
struct FieldCfg {
    long: Option<String>,
    short: Option<char>,
    action: Action,
    value_enum: bool,
    require_equals: bool,
    value_delimiter: Option<char>,
    flatten: bool,
    subcommand: bool,
    skip: bool,
    last: bool,
    trailing_var_arg: bool,
}
impl FieldCfg {
    fn parse(attrs: &[Attribute], field_name: &str, rename_all: &str) -> syn::Result<Self> {
        let mut cfg = Self::default();
        for attr in attrs {
            if attr.path().is_ident("arg") {
                for meta in parse_meta_list(attr)? {
                    match meta {
                        Meta::Path(p) if p.is_ident("long") => {
                            cfg.long = Some(rename(field_name, rename_all))
                        }
                        Meta::Path(p) if p.is_ident("short") => {
                            cfg.short = field_name.chars().next()
                        }
                        Meta::Path(p) if p.is_ident("value_enum") => cfg.value_enum = true,
                        Meta::Path(p) if p.is_ident("skip") => cfg.skip = true,
                        Meta::Path(p) if p.is_ident("require_equals") => cfg.require_equals = true,
                        Meta::Path(p) if p.is_ident("last") => cfg.last = true,
                        Meta::Path(p) if p.is_ident("trailing_var_arg") || p.is_ident("raw") => {
                            cfg.trailing_var_arg = true
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("long") => {
                            cfg.long = expr_string(&nv.value)
                                .or_else(|| Some(rename(field_name, rename_all)))
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("short") => {
                            cfg.short = expr_char(&nv.value).or_else(|| field_name.chars().next())
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("value_delimiter") => {
                            cfg.value_delimiter = expr_char(&nv.value)
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("require_equals") => {
                            cfg.require_equals = expr_bool(&nv.value).unwrap_or(true)
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("last") => {
                            cfg.last = expr_bool(&nv.value).unwrap_or(true)
                        }
                        Meta::NameValue(nv)
                            if nv.path.is_ident("trailing_var_arg") || nv.path.is_ident("raw") =>
                        {
                            cfg.trailing_var_arg = expr_bool(&nv.value).unwrap_or(true)
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("action") => {
                            cfg.action = parse_action(&nv.value)
                        }
                        Meta::NameValue(nv) if nv.path.is_ident("skip") => cfg.skip = true,
                        _ => {}
                    }
                }
            } else if attr.path().is_ident("command") {
                for meta in parse_meta_list(attr)? {
                    match meta {
                        Meta::Path(p) if p.is_ident("flatten") => cfg.flatten = true,
                        Meta::Path(p) if p.is_ident("subcommand") => cfg.subcommand = true,
                        Meta::Path(p) if p.is_ident("skip") => cfg.skip = true,
                        _ => {}
                    }
                }
            }
        }
        Ok(cfg)
    }
}

struct TypeShape<'a> {
    optional: bool,
    vector: bool,
    inner: &'a Type,
}
fn type_shape(ty: &Type) -> TypeShape<'_> {
    if let Some(inner) = generic_inner(ty, "Option") {
        if let Some(item) = generic_inner(inner, "Vec") {
            return TypeShape {
                optional: true,
                vector: true,
                inner: item,
            };
        }
        return TypeShape {
            optional: true,
            vector: false,
            inner,
        };
    }
    if let Some(inner) = generic_inner(ty, "Vec") {
        return TypeShape {
            optional: false,
            vector: true,
            inner,
        };
    }
    TypeShape {
        optional: false,
        vector: false,
        inner: ty,
    }
}
fn generic_inner<'a>(ty: &'a Type, expected: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else { return None };
    let seg = path.path.segments.last()?;
    if seg.ident != expected {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &seg.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| {
        if let GenericArgument::Type(ty) = arg {
            Some(ty)
        } else {
            None
        }
    })
}
fn is_bool(ty: &Type) -> bool {
    matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.segments.last().is_some_and(|s| s.ident == "bool"))
}

fn emit_field(
    access: TokenStream2,
    ty: &Type,
    cfg: &FieldCfg,
    runtime: &TokenStream2,
) -> syn::Result<TokenStream2> {
    if cfg.skip {
        return Ok(TokenStream2::new());
    }
    let shape = type_shape(ty);
    if cfg.flatten || cfg.subcommand {
        return Ok(if shape.optional {
            quote! { if let Some(__outboard_value) = #access { #runtime::ToArgv::append_argv(__outboard_value, __outboard_argv); } }
        } else if shape.vector {
            quote! { for __outboard_value in #access { #runtime::ToArgv::append_argv(__outboard_value, __outboard_argv); } }
        } else {
            quote! { #runtime::ToArgv::append_argv(#access, __outboard_argv); }
        });
    }
    let flag = cfg
        .long
        .as_ref()
        .map(|v| format!("--{v}"))
        .or_else(|| cfg.short.map(|v| format!("-{v}")));
    let positional = flag.is_none();
    let action = if cfg.action == Action::Default
        && !shape.optional
        && !shape.vector
        && is_bool(shape.inner)
        && !positional
    {
        Action::SetTrue
    } else {
        cfg.action
    };
    if matches!(action, Action::SetTrue | Action::SetFalse) {
        if shape.optional || shape.vector || !is_bool(shape.inner) {
            return Err(syn::Error::new_spanned(
                ty,
                "SetTrue/SetFalse requires a plain bool",
            ));
        }
        let flag = flag.ok_or_else(|| {
            syn::Error::new_spanned(ty, "boolean action requires a short or long flag")
        })?;
        return Ok(if action == Action::SetTrue {
            quote! { if *(#access) { __outboard_argv.push(#flag.into()); } }
        } else {
            quote! { if !*(#access) { __outboard_argv.push(#flag.into()); } }
        });
    }
    if action == Action::Count {
        let flag =
            flag.ok_or_else(|| syn::Error::new_spanned(ty, "Count requires a short or long flag"))?;
        return Ok(
            quote! { for _ in 0..(*( #access ) as usize) { __outboard_argv.push(#flag.into()); } },
        );
    }
    let separator = positional && (cfg.last || cfg.trailing_var_arg);
    if shape.optional && shape.vector {
        let body = emit_vector(
            quote!(__outboard_values),
            flag.as_deref(),
            cfg,
            runtime,
            separator,
        );
        return Ok(quote! { if let Some(__outboard_values) = #access { #body } });
    }
    if shape.optional {
        let body = emit_one(
            quote!(__outboard_value),
            flag.as_deref(),
            cfg,
            runtime,
            separator,
        );
        return Ok(quote! { if let Some(__outboard_value) = #access { #body } });
    }
    if shape.vector {
        return Ok(emit_vector(
            access,
            flag.as_deref(),
            cfg,
            runtime,
            separator,
        ));
    }
    Ok(emit_one(access, flag.as_deref(), cfg, runtime, separator))
}

fn emit_vector(
    values: TokenStream2,
    flag: Option<&str>,
    cfg: &FieldCfg,
    runtime: &TokenStream2,
    separator: bool,
) -> TokenStream2 {
    let sep = separator.then(|| quote! { __outboard_argv.push(::std::ffi::OsString::from("--")); });
    if let Some(delim) = cfg.value_delimiter {
        let delim = delim.to_string();
        let convert = conversion(quote!(__outboard_value), cfg.value_enum, runtime);
        let push = push_value(quote!(__outboard_joined), flag, cfg.require_equals, runtime);
        quote! { if !(#values).is_empty() { #sep let mut __outboard_joined=::std::ffi::OsString::new(); let mut __outboard_first=true; for __outboard_value in #values { if !__outboard_first { __outboard_joined.push(#delim); } __outboard_first=false; __outboard_joined.push(#convert); } #push } }
    } else {
        let convert = conversion(quote!(__outboard_value), cfg.value_enum, runtime);
        let push = push_value(quote!(#convert), flag, cfg.require_equals, runtime);
        quote! { if !(#values).is_empty() { #sep } for __outboard_value in #values { #push } }
    }
}
fn emit_one(
    value: TokenStream2,
    flag: Option<&str>,
    cfg: &FieldCfg,
    runtime: &TokenStream2,
    separator: bool,
) -> TokenStream2 {
    let convert = conversion(value, cfg.value_enum, runtime);
    let push = push_value(quote!(#convert), flag, cfg.require_equals, runtime);
    let sep = separator.then(|| quote! { __outboard_argv.push(::std::ffi::OsString::from("--")); });
    quote! { #sep #push }
}
fn conversion(value: TokenStream2, value_enum: bool, runtime: &TokenStream2) -> TokenStream2 {
    if value_enum {
        quote! { #runtime::value_enum_to_os(#value) }
    } else {
        quote! { #runtime::ToArgValue::to_arg_value(#value) }
    }
}
fn push_value(
    value: TokenStream2,
    flag: Option<&str>,
    equals: bool,
    runtime: &TokenStream2,
) -> TokenStream2 {
    match flag {
        Some(flag) => quote! { #runtime::push_flag_value(__outboard_argv,#flag,#value,#equals); },
        None => quote! { __outboard_argv.push(#value); },
    }
}
fn parse_meta_list(attr: &Attribute) -> syn::Result<Punctuated<Meta, Token![,]>> {
    attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
}
fn parse_action(expr: &Expr) -> Action {
    let s = quote!(#expr).to_string().replace(' ', "");
    if s.ends_with("ArgAction::SetTrue") || s == "SetTrue" {
        Action::SetTrue
    } else if s.ends_with("ArgAction::SetFalse") || s == "SetFalse" {
        Action::SetFalse
    } else if s.ends_with("ArgAction::Count") || s == "Count" {
        Action::Count
    } else if s.ends_with("ArgAction::Set")
        || s.ends_with("ArgAction::Append")
        || s == "Set"
        || s == "Append"
    {
        Action::Value
    } else {
        Action::Default
    }
}
fn expr_string(expr: &Expr) -> Option<String> {
    if let Expr::Lit(v) = expr {
        if let Lit::Str(v) = &v.lit {
            return Some(v.value());
        }
    }
    None
}
fn expr_char(expr: &Expr) -> Option<char> {
    if let Expr::Lit(v) = expr {
        match &v.lit {
            Lit::Char(v) => return Some(v.value()),
            Lit::Str(v) => return v.value().chars().next(),
            _ => {}
        }
    }
    None
}
fn expr_bool(expr: &Expr) -> Option<bool> {
    if let Expr::Lit(v) = expr {
        if let Lit::Bool(v) = &v.lit {
            return Some(v.value());
        }
    }
    None
}
fn container_rename_all(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs.iter().filter(|a| a.path().is_ident("command")) {
        if let Ok(metas) = parse_meta_list(attr) {
            for meta in metas {
                if let Meta::NameValue(nv) = meta {
                    if nv.path.is_ident("rename_all") {
                        if let Some(v) = expr_string(&nv.value) {
                            return Some(v);
                        }
                    }
                }
            }
        }
    }
    None
}
fn variant_name(attrs: &[Attribute], ident: &str, rename_all: &str) -> syn::Result<String> {
    for attr in attrs.iter().filter(|a| a.path().is_ident("command")) {
        for meta in parse_meta_list(attr)? {
            if let Meta::NameValue(nv) = meta {
                if nv.path.is_ident("name") {
                    if let Some(v) = expr_string(&nv.value) {
                        return Ok(v);
                    }
                }
            }
        }
    }
    Ok(rename(ident, rename_all))
}
fn rename(name: &str, style: &str) -> String {
    let words = words(name);
    match style {
        "kebab-case" => words.join("-"),
        "snake_case" => words.join("_"),
        "SCREAMING_SNAKE_CASE" => words.join("_").to_ascii_uppercase(),
        "lower" | "lowercase" => words.join("").to_ascii_lowercase(),
        "UPPER" | "UPPERCASE" => words.join("").to_ascii_uppercase(),
        "camelCase" => {
            let mut i = words.into_iter();
            let first = i.next().unwrap_or_default();
            first + &i.map(|w| capitalize(&w)).collect::<String>()
        }
        "PascalCase" => words.into_iter().map(|w| capitalize(&w)).collect(),
        "verbatim" => name.to_owned(),
        _ => words.join("-"),
    }
}
fn words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, ch) in chars.iter().copied().enumerate() {
        if ch == '_' || ch == '-' {
            if !cur.is_empty() {
                out.push(cur.to_ascii_lowercase());
                cur.clear()
            }
            continue;
        }
        let boundary = ch.is_ascii_uppercase()
            && !cur.is_empty()
            && (chars
                .get(i.wrapping_sub(1))
                .is_some_and(|p| p.is_ascii_lowercase())
                || chars.get(i + 1).is_some_and(|n| n.is_ascii_lowercase()));
        if boundary {
            out.push(cur.to_ascii_lowercase());
            cur.clear()
        }
        cur.push(ch)
    }
    if !cur.is_empty() {
        out.push(cur.to_ascii_lowercase())
    }
    out
}
fn capitalize(word: &str) -> String {
    let mut c = word.chars();
    c.next().map_or_else(String::new, |f| {
        f.to_ascii_uppercase().to_string() + c.as_str()
    })
}
