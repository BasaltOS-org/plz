use serde::{Deserialize, Serialize};
use settings::OriginKind;
use snafu::{OptionExt, ResultExt};
use std::{
    fs::File,
    io::{ErrorKind, Read, Write},
    path::Path,
};
use utils::{
    errors::{HowError, IOAction, IOSnafu, SystemSnafu, WhereError, YAMLSnafu},
    get_metadata_dir,
};

use crate::{DepVer, FuckNest, FuckWrap, MetaDataKind, Specific, processed::PreBuilt};

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
    pub fn open(name: &str) -> Result<Self, WhereError> {
        let mut path = get_metadata_dir().nest("Get Metadata Directory")?;
        path.push(format!("{}.yaml", name));
        let mut file = File::open(&path)
            .context(IOSnafu {
                action: IOAction::OpenFile,
                loc: path.display().to_string(),
            })
            .wrap()?;
        let mut metadata = String::new();
        file.read_to_string(&mut metadata)
            .context(IOSnafu {
                action: IOAction::ReadFile,
                loc: path.display().to_string(),
            })
            .wrap()?;
        serde_norway::from_str::<Self>(&metadata)
            .context(YAMLSnafu {
                loc: path.display().to_string(),
            })
            .wrap()
    }
    pub fn write(self, path: &Path) -> Result<Option<Self>, WhereError> {
        if !path.exists() || path.is_file() {
            let data = serde_norway::to_string(&self)
                .context(YAMLSnafu {
                    loc: self.name.to_string(),
                })
                .wrap()?;
            let mut file = File::create(path)
                .context(IOSnafu {
                    action: IOAction::CreateFile,
                    loc: path.display().to_string(),
                })
                .wrap()?;
            file.write_all(data.as_bytes())
                .context(IOSnafu {
                    action: IOAction::WriteFile,
                    loc: path.display().to_string(),
                })
                .wrap()?;
            Ok(Some(self))
        } else {
            Err(HowError::IOError {
                source: ErrorKind::NotSeekable.into(),
                action: IOAction::AssertPath,
                loc: path.display().to_string().into(),
            })
            .wrap()
        }
    }
    pub fn clear_dependencies(&self, specific: &Specific) -> Result<(), WhereError> {
        let mut path = get_metadata_dir().nest("Get Metadata Directory")?;
        let mut data = self.clone();
        let index = &data
            .dependencies
            .iter()
            .position(|x| x.get_installed_specific().is_ok_and(|x| x == *specific))
            .context(SystemSnafu {
                message: format!("Dependent `{}` {} not found", data.name, data.version),
                package: specific.name.to_string(),
            })
            .wrap()?;
        data.dependencies.remove(*index);
        path.push(format!("{}.yaml", self.name));
        data.write(&path)
            .nest("Write Changes to Package Metadata")?;
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
