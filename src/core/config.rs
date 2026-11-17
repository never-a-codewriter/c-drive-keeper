use anyhow::{Context,Result};
use serde::{Deserialize,Serialize};
use std::{path::{Path,PathBuf},hash::{Hash,Hasher}};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW,GetDriveTypeW};

#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct ScanSource {
    pub enabled:bool,
    pub label:String,
    #[serde(default)] pub path:PathBuf,
    pub target_subdir:String,
    #[serde(default,skip_serializing_if="Option::is_none")] pub known_folder:Option<String>,
}
#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct AppConfig {
    pub version:u32,
    #[serde(default)] pub target_root:PathBuf,
    #[serde(default="yes")] pub target_auto:bool,
    #[serde(default)] pub scan_sources:Vec<ScanSource>,
    #[serde(default)] pub environment_id:String,
    #[serde(skip)] pub startup_notice:String,
}
fn yes()->bool{true}
fn normalized(path:&Path)->String{path.to_string_lossy().replace('/',"\\").trim_end_matches('\\').to_lowercase()}
fn choose_target(drives:&[(PathBuf,u64)],system:&Path,documents:&Path)->PathBuf {
    drives.iter().filter(|(p,_)|normalized(p)!=normalized(system)).max_by_key(|(_,free)|*free)
        .map(|(p,_)|p.join("CDriveKeeper")).unwrap_or_else(||documents.join("CDriveKeeper"))
}
fn volume_exists(path:&Path)->bool {path.is_absolute()&&path.ancestors().last().map(|p|p.exists()).unwrap_or(false)}
pub fn system_volume()->PathBuf {
    if let Ok(drive)=std::env::var("SystemDrive"){return PathBuf::from(format!("{}\\",drive.trim_end_matches('\\')));}
    if let Ok(windows)=std::env::var("SystemRoot"){if let Some(root)=Path::new(&windows).ancestors().last(){return root.into();}}
    directories::BaseDirs::new().and_then(|d|d.home_dir().ancestors().last().map(Path::to_path_buf)).unwrap_or_default()
}
impl AppConfig {
    const VERSION:u32=2;
    pub fn config_path()->Result<PathBuf>{Ok(std::env::current_exe()?.parent().context("无法定位程序目录")?.join("config.json"))}
    pub fn load_or_create()->Result<Self>{
        let path=Self::config_path()?;let defaults=Self::default_config();
        let mut cfg=if path.exists(){
            let raw=std::fs::read_to_string(&path)?;
            match serde_json::from_str::<Self>(raw.trim_start_matches('\u{feff}')){
                Ok(cfg)=>cfg,
                Err(_)=>{std::fs::copy(&path,path.with_extension(format!("invalid-{}.json",uuid::Uuid::new_v4())))?;defaults.clone()}
            }
        }else{defaults.clone()};
        cfg.rebind(&defaults);cfg.save()?;Ok(cfg)
    }
    fn rebind(&mut self,defaults:&Self){
        let legacy=self.version<Self::VERSION;
        let different=!self.environment_id.is_empty()&&self.environment_id!=defaults.environment_id;
        let mut notices=Vec::new();
        if legacy {self.target_auto=true;notices.push("旧版固定目录配置已升级为动态识别，可重新选择目标目录");}
        if different {self.target_auto=true;notices.push("检测到电脑或用户变化：已刷新内置目录，自定义扫描目录已停用，请重新选择");}
        for source in &mut self.scan_sources {
            if source.known_folder.is_none() && legacy {
                source.known_folder=match source.label.as_str(){"LocalAppData"=>Some("LocalAppData".into()),"RoamingAppData"=>Some("RoamingAppData".into()),_=>None};
            }
            if let Some(kind)=&source.known_folder {
                if let Some(current)=defaults.scan_sources.iter().find(|s|s.known_folder.as_ref()==Some(kind)){source.path=current.path.clone();}
                else {source.enabled=false;source.path=PathBuf::new();}
            }else if legacy || different || !source.path.is_dir(){source.enabled=false;}
        }
        for source in &defaults.scan_sources {if !self.scan_sources.iter().any(|s|s.known_folder==source.known_folder){self.scan_sources.push(source.clone());}}
        if self.target_auto || !volume_exists(&self.target_root){
            if !self.target_auto{notices.push("原目标盘不可用，已改为自动选择；执行前请核对目标");}
            self.target_auto=true;self.target_root=defaults.target_root.clone();
        }
        self.version=Self::VERSION;self.environment_id=defaults.environment_id.clone();self.startup_notice=notices.join("；");
    }
    fn portable(&self)->Self{
        let mut saved=self.clone();
        for source in &mut saved.scan_sources {if source.known_folder.is_some(){source.path=PathBuf::new();}}
        if saved.target_auto{saved.target_root=PathBuf::new();}saved
    }
    pub fn save(&self)->Result<()>{std::fs::write(Self::config_path()?,serde_json::to_string_pretty(&self.portable())?).context("无法保存配置")}
    pub fn default_config()->Self{
        let mut sources=Vec::new();let base=directories::BaseDirs::new();
        if let Some(base)=&base{
            for (kind,path,sub) in [("LocalAppData",base.data_local_dir(),"Local"),("RoamingAppData",base.data_dir(),"Roaming")]{
                sources.push(ScanSource{enabled:true,label:kind.into(),path:path.into(),target_subdir:sub.into(),known_folder:Some(kind.into())});
            }
        }
        let documents=directories::UserDirs::new().map(|u|u.document_dir().unwrap_or(u.home_dir()).to_path_buf()).unwrap_or_else(||std::env::current_dir().unwrap_or_default());
        let mut drives=Vec::new();
        for letter in b'A'..=b'Z'{let path=PathBuf::from(format!("{}:\\",letter as char));let wide:Vec<u16>=path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();unsafe{
            if GetDriveTypeW(PCWSTR(wide.as_ptr()))!=3{continue;}let mut free=0;if GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()),Some(&mut free),None,None).as_bool(){drives.push((path,free));}
        }}
        let mut hash=std::collections::hash_map::DefaultHasher::new();std::env::var("COMPUTERNAME").unwrap_or_default().hash(&mut hash);if let Some(base)=&base{base.home_dir().hash(&mut hash);}
        Self{version:Self::VERSION,target_root:choose_target(&drives,&system_volume(),&documents),target_auto:true,scan_sources:sources,environment_id:format!("{:016x}",hash.finish()),startup_notice:String::new()}
    }
    pub fn add_custom_scan_dir(&mut self,path:&Path){
        let label=path.file_name().and_then(|n|n.to_str()).unwrap_or("Custom").to_string();let mut sub=label.clone();let mut i=2;
        while self.scan_sources.iter().any(|s|s.target_subdir.eq_ignore_ascii_case(&sub)){sub=format!("{label}_{i}");i+=1;}
        self.scan_sources.push(ScanSource{enabled:true,label,path:path.into(),target_subdir:sub,known_folder:None});
    }
}
#[cfg(test)] mod tests{
    use super::*;
    fn defaults()->AppConfig{AppConfig{version:2,target_root:PathBuf::from("E:\\CDriveKeeper"),target_auto:true,scan_sources:vec![ScanSource{enabled:true,label:"LocalAppData".into(),path:PathBuf::from("F:\\Profiles\\NewUser\\Local"),target_subdir:"Local".into(),known_folder:Some("LocalAppData".into())}],environment_id:"new-machine".into(),startup_notice:String::new()}}
    #[test]fn no_d_drive_is_required(){assert_eq!(choose_target(&[(PathBuf::from("C:\\"),999),(PathBuf::from("E:\\"),20)],Path::new("c:\\"),Path::new("F:\\Docs")),PathBuf::from("E:\\CDriveKeeper"));}
    #[test]fn system_only_uses_current_documents(){assert_eq!(choose_target(&[(PathBuf::from("C:\\"),999)],Path::new("C:\\"),Path::new("F:\\Docs")),PathBuf::from("F:\\Docs\\CDriveKeeper"));}
    #[test]fn builtins_and_auto_target_serialize_without_absolute_paths(){let cfg=defaults();let saved=cfg.portable();assert!(saved.target_root.as_os_str().is_empty());assert!(saved.scan_sources[0].path.as_os_str().is_empty());}
    #[test]fn old_user_path_rebinds_to_current_known_folder(){let mut cfg:AppConfig=serde_json::from_str(r#"{"version":1,"target_root":"D:\\old","scan_sources":[{"enabled":true,"label":"LocalAppData","path":"C:\\Users\\OldUser\\AppData\\Local","target_subdir":"Local"}]}"#).unwrap();cfg.rebind(&defaults());assert_eq!(cfg.scan_sources[0].path,defaults().scan_sources[0].path);assert_eq!(cfg.target_root,defaults().target_root);}
    #[test]fn custom_paths_disabled_on_other_computer(){let mut cfg=defaults();cfg.environment_id="old-machine".into();cfg.scan_sources.push(ScanSource{enabled:true,label:"custom".into(),path:std::env::temp_dir(),target_subdir:"custom".into(),known_folder:None});cfg.rebind(&defaults());assert!(!cfg.scan_sources.last().unwrap().enabled);}
    #[test]fn missing_target_volume_falls_back(){let mut cfg=defaults();cfg.target_auto=false;cfg.target_root=PathBuf::from("not-absolute");cfg.rebind(&defaults());assert!(cfg.target_auto);assert_eq!(cfg.target_root,defaults().target_root);}
}

