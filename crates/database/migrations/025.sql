CREATE TABLE notification_conditions(source TEXT PRIMARY KEY, occurrence INTEGER NOT NULL, active INTEGER NOT NULL);
CREATE TRIGGER users_keep_admin_delete BEFORE DELETE ON users
WHEN OLD.role='admin' AND (SELECT COUNT(*) FROM users WHERE role='admin')<=1 BEGIN
 SELECT RAISE(ABORT,'Keep at least one administrator');
END;
CREATE TRIGGER users_keep_admin_role BEFORE UPDATE OF role ON users
WHEN OLD.role='admin' AND NEW.role<>'admin' AND (SELECT COUNT(*) FROM users WHERE role='admin')<=1 BEGIN
 SELECT RAISE(ABORT,'Keep at least one administrator');
END;
