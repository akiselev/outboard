use std::{collections::BTreeMap, fs, path::{Path, PathBuf}};
use crate::{Namespace, PluginId, PluginKind, PluginName};

#[derive(Debug, Clone, PartialEq, Eq)] pub enum DiscoverySource { Override, ExplicitPath, Environment(String), Path }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct SearchRoot { pub path: PathBuf, pub source: DiscoverySource }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct PluginCandidate { pub id: PluginId, pub path: PathBuf, pub source: DiscoverySource, pub precedence: usize }
impl PluginCandidate { pub fn explicit(id: PluginId, path: impl Into<PathBuf>) -> Self { Self { id, path:path.into(), source:DiscoverySource::Override, precedence:0 } } }
#[derive(Debug, Clone)] pub struct DiscoverySet { pub selected: Vec<PluginCandidate>, pub shadowed: BTreeMap<PluginId,Vec<PluginCandidate>>, pub roots: Vec<SearchRoot> }
impl DiscoverySet {
    pub fn selected_by_name(&self,name:&PluginName)->Option<&PluginCandidate>{self.selected.iter().find(|c|&c.id.name==name)}
    pub fn all_for<'a>(&'a self,id:&'a PluginId)->impl Iterator<Item=&'a PluginCandidate>{self.selected.iter().filter(move|c|&c.id==id).chain(self.shadowed.get(id).into_iter().flatten())}
}
pub(crate) fn discover_in_roots(namespace:&Namespace,kind:&PluginKind,roots:Vec<SearchRoot>,overrides:impl IntoIterator<Item=(PluginName,PathBuf)>)->Result<DiscoverySet,std::io::Error>{
    let mut selected=BTreeMap::new(); let mut shadowed=BTreeMap::new(); let mut precedence=0;
    for(name,path)in overrides{let id=PluginId{namespace:namespace.clone(),kind:kind.clone(),name};insert(&mut selected,&mut shadowed,PluginCandidate{id,path,source:DiscoverySource::Override,precedence});precedence+=1;}
    let prefix=format!("{}-{}-",namespace,kind);
    for root in &roots{
        let rd=match fs::read_dir(&root.path){Ok(v)=>v,Err(e)if matches!(e.kind(),std::io::ErrorKind::NotFound|std::io::ErrorKind::PermissionDenied)=>continue,Err(e)=>return Err(e)};
        let mut entries:Vec<_>=rd.flatten().collect(); entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries{let path=entry.path();if !is_executable(&path){continue}let Some(stem)=executable_file_name(&path)else{continue};let Some(name)=stem.strip_prefix(&prefix)else{continue};let Ok(name)=PluginName::new(name)else{continue};let id=PluginId{namespace:namespace.clone(),kind:kind.clone(),name};insert(&mut selected,&mut shadowed,PluginCandidate{id,path,source:root.source.clone(),precedence});precedence+=1;}
    }
    let mut selected:Vec<_>=selected.into_values().collect();selected.sort_by_key(|c|c.precedence);Ok(DiscoverySet{selected,shadowed,roots})
}
fn insert(selected:&mut BTreeMap<PluginId,PluginCandidate>,shadowed:&mut BTreeMap<PluginId,Vec<PluginCandidate>>,candidate:PluginCandidate){if let Some(existing)=selected.get(&candidate.id){if same_file(&existing.path,&candidate.path){return}shadowed.entry(candidate.id.clone()).or_default().push(candidate);}else{selected.insert(candidate.id.clone(),candidate);}}
fn same_file(a:&Path,b:&Path)->bool{match(a.canonicalize(),b.canonicalize()){(Ok(a),Ok(b))=>a==b,_=>a==b}}
pub fn executable_file_name(path:&Path)->Option<String>{let name=path.file_name()?.to_str()?;#[cfg(windows)]{for ext in [".exe",".com",".cmd",".bat"]{let lower=name.to_ascii_lowercase();if lower.ends_with(ext){return Some(name[..name.len()-ext.len()].to_owned())}}}Some(name.to_owned())}
#[cfg(unix)]fn is_executable(path:&Path)->bool{use std::os::unix::fs::PermissionsExt;fs::metadata(path).map(|m|m.is_file()&&m.permissions().mode()&0o111!=0).unwrap_or(false)}
#[cfg(windows)]fn is_executable(path:&Path)->bool{if !path.is_file(){return false}let e=path.extension().and_then(std::ffi::OsStr::to_str).unwrap_or_default().to_ascii_lowercase();matches!(e.as_str(),"exe"|"com"|"cmd"|"bat")}
#[cfg(not(any(unix,windows)))]fn is_executable(path:&Path)->bool{path.is_file()}
pub(crate) fn build_roots(explicit:&[PathBuf],env:Option<&str>,include_path:bool)->Vec<SearchRoot>{let mut r=Vec::new();for p in explicit{r.push(SearchRoot{path:p.clone(),source:DiscoverySource::ExplicitPath})}if let Some(var)=env{if let Some(v)=std::env::var_os(var){for p in std::env::split_paths(&v){r.push(SearchRoot{path:p,source:DiscoverySource::Environment(var.into())})}}}if include_path{if let Some(v)=std::env::var_os("PATH"){for p in std::env::split_paths(&v){r.push(SearchRoot{path:p,source:DiscoverySource::Path})}}}let mut out=Vec::new();for x in r{if !out.iter().any(|e:&SearchRoot|same_file(&e.path,&x.path)){out.push(x)}}out}
