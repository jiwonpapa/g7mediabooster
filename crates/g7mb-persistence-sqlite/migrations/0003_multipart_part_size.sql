ALTER TABLE uploads ADD COLUMN multipart_part_size_bytes INTEGER
    CHECK (
        multipart_part_size_bytes IS NULL
        OR multipart_part_size_bytes >= 5242880
    );
