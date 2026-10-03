CREATE TABLE media_files_v5(
    id INTEGER PRIMARY KEY,
    source_id INTEGER NOT NULL REFERENCES sources(id),
    uri TEXT NOT NULL UNIQUE,
    relative TEXT NOT NULL,
    identity TEXT NOT NULL UNIQUE,
    size INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    data TEXT NOT NULL
);
INSERT INTO media_files_v5
    SELECT id,source_id,uri,relative,identity,size,modified,data FROM media_files;

CREATE TABLE source_files_v5(
    source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    file_id INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    relative TEXT NOT NULL,
    PRIMARY KEY(source_id,file_id)
);
INSERT INTO source_files_v5 SELECT source_id,file_id,relative FROM source_files;

DROP TABLE source_files;
DROP TABLE media_files;
ALTER TABLE media_files_v5 RENAME TO media_files;
ALTER TABLE source_files_v5 RENAME TO source_files;
