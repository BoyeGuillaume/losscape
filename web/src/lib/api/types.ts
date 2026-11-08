export type RunState = 'running' | 'finished' | 'failed' | 'killed' | 'crashed';

export type Role = 'owner' | 'editor' | 'viewer';

export type ProjectSummary = {
	name: string;
	role: Role;
	runs: number;
	last_activity_ms: number | null;
};

export type RunSummary = {
	id: string;
	name: string;
	state: RunState;
	group_name: string | null;
	tags: string[];
	aliases: string[];
	created_at_ms: number;
	/** Start on the client's clock (the clock of metric timestamps). */
	start_ms: number;
	/** Set on archived runs (deleted for good after the project's `archive_days`). */
	archived_at_ms: number | null;
	/** Highest step logged (system metrics aside); null before the first point. */
	last_step: number | null;
	/** Lowest and highest value of each logged metric (system metrics aside). */
	metric_min: Record<string, number>;
	metric_max: Record<string, number>;
	/** Metrics renamed in this run: logged name -> shown name. */
	renames: Record<string, string>;
	heartbeat_at_ms: number;
	finished_at_ms: number | null;
	summary: Record<string, unknown>;
	config: Record<string, unknown>;
};

export type MetricInfo = {
	key: string;
	points: number;
	min_step: number;
	max_step: number;
};

/** `POST /projects/{project}/metrics`: one row per (run, metric). */
export type RunMetricInfo = MetricInfo & { run: string };

export type RunDetail = RunSummary & {
	project: string;
	notes: string;
	exit_code: number | null;
	environment: RunEnvironment;
	metrics: MetricInfo[];
};

export type Me = {
	id: number;
	username: string;
	display_name: string | null;
	email: string | null;
	avatar_url: string | null;
	is_admin: boolean;
	has_password: boolean;
	identities: string[];
};

export type AuthSettings = {
	signup: boolean;
	first_user: boolean;
	providers: { id: string; name: string; icon: ProviderIconName }[];
};

export type ApiKey = {
	id: number;
	name: string;
	prefix: string;
	created_at_ms: number;
	last_used_at_ms: number | null;
};

export type Member = {
	username: string;
	display_name: string | null;
	avatar_url: string | null;
	role: Role;
	added_at_ms: number;
};

export type ProjectAccess = { name: string; role: Role; members: Member[] };

export type ProjectInfo = {
	name: string;
	role: Role;
	created_at_ms: number;
	/** Archived runs are deleted for good after this many days. */
	archive_days: number;
	runs: number;
	archived_runs: number;
};

export type ProviderIconName = 'google' | 'github' | 'microsoft' | 'gitlab' | 'key';

/** What the SDK captured about where a run ran (every field optional: older runs). */
export type RunEnvironment = {
	hostname?: string;
	command?: string;
	python_version?: string;
	sdk_version?: string;
	git?: { remote?: string; commit?: string; dirty?: boolean; diff_bytes?: number };
	git_branch?: string;
	git_untracked?: string[];
	python?: { executable?: string; implementation?: string };
	torch?: { version?: string | null; cuda?: string | null };
	os?: { name?: string | null; version?: string | null; kernel?: string | null; arch?: string };
	cpu?: {
		model?: string | null;
		cores?: number | null;
		threads?: number | null;
		memory_gb?: number;
	};
	gpu?: {
		driver?: string | null;
		cuda_driver?: string | null;
		devices?: { name?: string | null; memory_gb?: number | null }[];
	};
	slurm?: Record<string, string>;
	container?: { runtime?: string; image?: string | null };
	env?: Record<string, string>;
	packages?: string[];
	/** Console capture: whether it ran, and why not (`off`: console=False, LOSSCAPE_CONSOLE,
	 * windows, notebook; `error`: it failed to start). */
	console?: { captured?: boolean; off?: string; error?: string };
};
