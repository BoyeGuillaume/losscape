-- A project has exactly one owner; ownership moves only by transfer. Projects that had
-- several keep the earliest one, and the others become editors.
UPDATE project_members m SET role = 'editor'
WHERE m.role = 'owner' AND EXISTS (
    SELECT 1 FROM project_members o
    WHERE o.project_id = m.project_id AND o.role = 'owner'
      AND (o.added_at, o.user_id) < (m.added_at, m.user_id)
);
CREATE UNIQUE INDEX project_members_one_owner ON project_members (project_id)
    WHERE role = 'owner';
