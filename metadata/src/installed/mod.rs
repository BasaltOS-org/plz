use serde::{Deserialize, Serialize};
use settings::OriginKind;
use snafu::{OptionExt, ResultExt, Whatever, whatever};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
use utils::get_metadata_dir;

use crate::processed::PreBuilt;
use crate::{DepVer, MetaDataKind, Specific};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InstalledMetaData {
    pub name: String,
    pub kind: MetaDataKind,
    pub version: String,
    pub origin: OriginKind,
    pub dependent: bool,
    pub dependencies: Vec<DepVer>,
    pub dependents: Vec<Specific>,
    pub install_kind: InstalledInstallKind,
    pub hash: String,
}

impl InstalledMetaData {
    pub fn open(name: &str) -> Result<Self, Whatever> {
        let mut path = get_metadata_dir()?;
        path.push(format!("{}.yaml", name));
        let mut file = File::open(&path)
            .with_whatever_context(|_| format!("Failed to read package `{name}`'s metadata!"))?;
        let mut metadata = String::new();
        file.read_to_string(&mut metadata)
            .with_whatever_context(|_| format!("Failed to read package `{name}`'s config!"))?;
        serde_norway::from_str::<Self>(&metadata)
            .with_whatever_context(|_| format!("Failed to parse package `{name}`'s data!"))
    }
    pub fn write(self, path: &Path) -> Result<Option<Self>, Whatever> {
        if !path.exists() || path.is_file() {
            let data = serde_norway::to_string(&self).with_whatever_context(|_| {
                format!(
                    "Failed to parse `{}`'s InstalledMetaData into string!",
                    self.name
                )
            })?;
            let mut file = File::create(path).with_whatever_context(|_| {
                format!("Failed to open file for `{}` as WO!", self.name)
            })?;
            file.write_all(data.as_bytes())
                .with_whatever_context(|_| format!("Failed to write `{}` to file!", self.name))?;
            Ok(Some(self))
        } else {
            whatever!("File is of unexpected type!")
        }
    }
    pub fn clear_dependencies(&self, specific: &Specific) -> Result<(), Whatever> {
        let mut path = get_metadata_dir()?;
        let mut data = self.clone();
        let index = &data
            .dependencies
            .iter()
            .position(|x| x.get_installed_specific().is_ok_and(|x| x == *specific))
            .with_whatever_context(|| {
                format!(
                    "`{}` {} didn't contain dependent `{}`!",
                    data.name, data.version, specific.name
                )
            })?;
        data.dependencies.remove(*index);
        path.push(format!("{}.yaml", self.name));
        data.write(&path)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum InstalledInstallKind {
    PreBuilt(PreBuilt),
    Compilable(InstalledCompilable),
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InstalledCompilable {
    pub uninstall: String,
    pub purge: String,
}
