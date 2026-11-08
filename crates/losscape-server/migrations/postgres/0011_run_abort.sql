-- Someone asked for this running run to stop (the UI's Abort): its SDK learns about it from
-- its next heartbeat, and interrupts the script.
ALTER TABLE runs ADD COLUMN abort_requested_at TIMESTAMPTZ;
