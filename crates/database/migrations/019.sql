ALTER TABLE stack_provisions ADD COLUMN origin TEXT NOT NULL DEFAULT 'installed' CHECK(origin IN ('installed','adopted'));
ALTER TABLE stack_provisions ADD COLUMN native_url TEXT NOT NULL DEFAULT '';
