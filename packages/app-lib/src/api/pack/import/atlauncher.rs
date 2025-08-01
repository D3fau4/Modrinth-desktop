use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    State,
    pack::{
        self,
        import::{self, copy_dotminecraft},
        install_from::CreatePackDescription,
    },
    prelude::ModLoader,
    state::{LinkedData, ProfileInstallStage},
    util::io,
};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ATInstance {
    pub id: String, // minecraft version id ie: 1.12.1, not a name
    pub launcher: ATLauncher,
    pub java_version: ATJavaVersion,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ATLauncher {
    pub name: String,
    pub pack: String,
    pub version: String, // ie: 1.6
    pub loader_version: ATLauncherLoaderVersion,

    pub modrinth_project: Option<ATLauncherModrinthProject>,
    pub modrinth_version: Option<ATLauncherModrinthVersion>,
    pub modrinth_manifest: Option<pack::install_from::PackFormat>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ATJavaVersion {
    pub major_version: u8,
    pub component: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ATLauncherLoaderVersion {
    pub r#type: String,
    pub version: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ATLauncherModrinthProject {
    pub id: String,
    pub slug: String,
    pub project_type: String,
    pub team: String,
    pub client_side: Option<String>,
    pub server_side: Option<String>,
    pub categories: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ATLauncherModrinthVersion {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ATLauncherModrinthVersionFile {
    pub hashes: HashMap<String, String>,
    pub url: String,
    pub filename: String,
    pub primary: bool,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ATLauncherModrinthVersionDependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ATLauncherMod {
    pub name: String,
    pub version: String,
    pub file: String,

    pub modrinth_project: Option<ATLauncherModrinthProject>,
    pub modrinth_version: Option<ATLauncherModrinthVersion>,
}

// Check if folder has a instance.json that parses
pub async fn is_valid_atlauncher(instance_folder: PathBuf) -> bool {
    let instance = serde_json::from_str::<ATInstance>(
        &io::read_any_encoding_to_string(
            &instance_folder.join("instance.json"),
        )
        .await
        .unwrap_or(("".into(), encoding_rs::UTF_8))
        .0,
    );

    if let Err(e) = instance {
        tracing::warn!(
            "Could not parse instance.json at {}: {}",
            instance_folder.display(),
            e
        );
        false
    } else {
        true
    }
}

#[tracing::instrument]

pub async fn import_atlauncher(
    atlauncher_base_path: PathBuf, // path to base atlauncher folder
    instance_folder: String,       // instance folder in atlauncher_base_path
    profile_path: &str,            // path to profile
) -> crate::Result<()> {
    let atlauncher_instance_path = atlauncher_base_path
        .join("instances")
        .join(&instance_folder);

    // Load instance.json
    let atinstance = serde_json::from_str::<ATInstance>(
        &io::read_any_encoding_to_string(
            &atlauncher_instance_path.join("instance.json"),
        )
        .await
        .unwrap_or(("".into(), encoding_rs::UTF_8))
        .0,
    )?;

    // Icon path should be {instance_folder}/instance.png if it exists,
    // Second possibility is ATLauncher/configs/images/{safe_pack_name}.png (safe pack name is alphanumeric lowercase)
    let icon_path_primary = atlauncher_instance_path.join("instance.png");
    let safe_pack_name = atinstance
        .launcher
        .pack
        .replace(|c: char| !c.is_alphanumeric(), "")
        .to_lowercase();
    let icon_path_secondary = atlauncher_base_path
        .join("configs")
        .join("images")
        .join(safe_pack_name + ".png");
    let icon = match (icon_path_primary.exists(), icon_path_secondary.exists())
    {
        (true, _) => import::recache_icon(icon_path_primary).await?,
        (_, true) => import::recache_icon(icon_path_secondary).await?,
        _ => None,
    };

    // Create description from instance.cfg
    let description = CreatePackDescription {
        icon,
        override_title: Some(atinstance.launcher.name.clone()),
        project_id: None,
        version_id: None,
        existing_loading_bar: None,
        profile_path: profile_path.to_string(),
    };

    let backup_name = format!("ATLauncher-{instance_folder}");
    let minecraft_folder = atlauncher_instance_path;

    import_atlauncher_unmanaged(
        profile_path,
        minecraft_folder,
        backup_name,
        description,
        atinstance,
    )
    .await?;
    Ok(())
}

async fn import_atlauncher_unmanaged(
    profile_path: &str,
    minecraft_folder: PathBuf,
    backup_name: String,
    description: CreatePackDescription,
    atinstance: ATInstance,
) -> crate::Result<()> {
    let mod_loader = format!(
        "\"{}\"",
        atinstance.launcher.loader_version.r#type.to_lowercase()
    );
    let mod_loader: ModLoader = serde_json::from_str::<ModLoader>(&mod_loader)
        .map_err(|_| {
            crate::ErrorKind::InputError(format!(
                "Could not parse mod loader type: {mod_loader}"
            ))
        })?;

    let game_version = atinstance.id;

    let loader_version = if mod_loader != ModLoader::Vanilla {
        crate::launcher::get_loader_version_from_profile(
            &game_version,
            mod_loader,
            Some(&atinstance.launcher.loader_version.version),
        )
        .await?
    } else {
        None
    };

    // Set profile data to created default profile
    crate::api::profile::edit(profile_path, |prof| {
        prof.name = description
            .override_title
            .clone()
            .unwrap_or_else(|| backup_name.to_string());
        prof.install_stage = ProfileInstallStage::PackInstalling;

        if let Some(ref project_id) = description.project_id {
            if let Some(ref version_id) = description.version_id {
                prof.linked_data = Some(LinkedData {
                    project_id: project_id.clone(),
                    version_id: version_id.clone(),
                    locked: true,
                })
            }
        }

        prof.icon_path = description
            .icon
            .clone()
            .map(|x| x.to_string_lossy().to_string());
        prof.game_version.clone_from(&game_version);
        prof.loader_version = loader_version.clone().map(|x| x.id);
        prof.loader = mod_loader;

        async { Ok(()) }
    })
    .await?;

    // Moves .minecraft folder over (ie: overrides such as resourcepacks, mods, etc)
    let state = State::get().await?;
    let loading_bar = copy_dotminecraft(
        profile_path,
        minecraft_folder,
        &state.io_semaphore,
        None,
    )
    .await?;

    if let Some(profile_val) = crate::api::profile::get(profile_path).await? {
        crate::launcher::install_minecraft(
            &profile_val,
            Some(loading_bar),
            false,
        )
        .await?;
    }
    Ok(())
}
