-- Schema 6 to 7. Runs inside the opening transaction with foreign keys off.

-- Music lists find and order a page of one kind by its shown title.
CREATE INDEX media_by_title ON media(kind,COALESCE(json_extract(metadata,'$.title'),json_extract(metadata,'$.name'),title) COLLATE NOCASE,id);
