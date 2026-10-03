mod documents;
mod imports;
mod migrations;
mod path_migration;
mod progress;
mod sources;

use super::*;

fn draft(root: &str, n: usize) -> Draft {
    Draft {
        root_uri: format!("file:///{root}"),
        title: root.into(),
        author: "".into(),
        include: true,
        files: (0..n)
            .map(|i| Media {
                uri: format!("file:///{root}/{i}.wav"),
                identity: format!("{root}/{i}"),
                relative: format!("{i}.wav"),
                title: format!("Part {i}"),
                duration: Some(60_000),
                size: Some(0),
                modified: Some(0),
                ..Default::default()
            })
            .collect(),
    }
}
