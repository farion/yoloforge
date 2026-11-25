use anyhow::{ensure, Context, Result};
use factory_protocol::skills::{valid_skill_files, FactorySkillBundle};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn install(registration: &Path, bundle: &FactorySkillBundle) -> Result<PathBuf> {
    let runtime = registration
        .parent()
        .context("OpenCode-Runtime fehlt")?
        .canonicalize()?;
    let root = runtime.join("config/opencode/skills");
    ensure!(
        fs::symlink_metadata(&root)?.is_dir() && root.canonicalize()? == root,
        "Factory-Skill-Verzeichnis ist nicht regulär"
    );
    ensure!(
        bundle.reference.is_valid() && valid_skill_files(&bundle.files),
        "Ungültiges Factory-Skill-Paket"
    );
    let dest = root.join(bundle.reference.runtime_id());
    let mut files = bundle.files.clone();
    let main = files.get_mut("SKILL.md").context("SKILL.md fehlt")?;
    *main = normalized(main, &bundle.name, &bundle.description)?;
    match fs::symlink_metadata(&dest) {
        Ok(meta) => ensure!(meta.is_dir(), "Factory-Skill-Pfad ist belegt"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let stage = tempfile::Builder::new()
                .prefix(".skill-")
                .tempdir_in(&root)?;
            for (name, body) in &files {
                let path = stage.path().join(name);
                fs::create_dir_all(path.parent().context("Skill-Pfad ohne Verzeichnis")?)?;
                fs::write(path, body)?;
            }
            fs::rename(stage.path(), &dest)?;
        }
        Err(error) => return Err(error.into()),
    }
    for (name, body) in files {
        let path = dest.join(&name);
        let meta = fs::symlink_metadata(&path)?;
        ensure!(
            meta.is_file()
                && path.canonicalize()? == path
                && meta.len() == body.len() as u64
                && fs::read(&path)? == body.as_bytes(),
            "Lokales Factory-Skill-Paket stimmt nicht mit der Version überein"
        );
    }
    Ok(dest.join("SKILL.md"))
}

fn normalized(markdown: &str, name: &str, description: &str) -> Result<String> {
    let markdown = markdown.trim_start_matches('\u{feff}');
    let mut metadata = serde_yaml::Mapping::new();
    let mut body = markdown;
    if let Some((first, rest)) = markdown.split_once('\n') {
        if first.trim_end() == "---" {
            let mut offset = 0;
            let (start, end) = rest
                .split_inclusive('\n')
                .find_map(|line| {
                    let start = offset;
                    offset += line.len();
                    matches!(line.trim_end(), "---" | "...").then_some((start, offset))
                })
                .context("Skill-Frontmatter nicht abgeschlossen")?;
            if !rest[..start].trim().is_empty() {
                metadata =
                    serde_yaml::from_str(&rest[..start]).context("Ungültige Skill-Frontmatter")?;
            }
            body = &rest[end..];
        }
    }
    metadata.insert("name".into(), name.into());
    metadata.insert("description".into(), description.into());
    Ok(format!(
        "---\n{}---\n{body}",
        serde_yaml::to_string(&metadata)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use factory_core::FactorySkillRef;

    #[test]
    fn installs_an_immutable_version_outside_the_repository() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().canonicalize().unwrap();
        fs::create_dir_all(runtime.join("config/opencode/skills")).unwrap();
        let registration = runtime.join("registration.json");
        let mut bundle = FactorySkillBundle {
            reference: FactorySkillRef {
                id: "review".into(),
                version: 1,
            },
            name: "Review".into(),
            description: "Review checklists".into(),
            files: [
                (
                    "SKILL.md".into(),
                    "---\nname: Old\nlicense: MIT\n---\nRead refs.\n".into(),
                ),
                ("refs/review.md".into(), "References".into()),
            ]
            .into(),
        };
        let file = install(&registration, &bundle).unwrap();
        assert!(file.ends_with("yf-catalog-review-v1/SKILL.md"));
        assert_eq!(install(&registration, &bundle).unwrap(), file);
        let content = fs::read_to_string(file).unwrap();
        assert!(content.contains("name: Review") && content.contains("license: MIT"));
        bundle
            .files
            .insert("refs/review.md".into(), "Changed".into());
        assert!(install(&registration, &bundle).is_err());
        bundle.reference.version = 2;
        assert!(install(&registration, &bundle)
            .unwrap()
            .ends_with("yf-catalog-review-v2/SKILL.md"));
    }
}
