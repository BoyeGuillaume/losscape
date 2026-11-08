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
	/** Someone asked this running run to stop; it does at its next heartbeat. */
	abort_requested_at_ms: number | null;
	renames: Record<string, string>;
	/** Factors of renamed metrics, when not 1. */
	rename_scales: Record<string, number>;
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

/** What `run.describe_model()` recorded. */
export type ModelTensor = {
	name: string;
	/** Null until a lazy module is initialized. */
	shape: number[] | null;
	trainable?: boolean;
	buffer?: boolean;
	shared_with?: string;
};
export type ModelNode = {
	name: string;
	class: string;
	/** In this module and below it, shared weights counted once. */
	params: number;
	trainable: number;
	extra?: string;
	input?: string;
	output?: string;
	calls?: number;
	/** Identical numbered neighbours shown as this one, up to the one named `last`. */
	repeat?: number;
	last?: string;
	/** All their names (older runs don't have it: they count up from this one's). */
	names?: string[];
	tensors?: ModelTensor[];
	children?: ModelNode[];
	/** Children left out of a very large model. */
	hidden_children?: number;
	shared_with?: string;
};
export type ModelDescription = {
	class: string;
	params: number;
	trainable_params: number;
	buffers: number;
	size_bytes: number;
	dtypes: string[];
	devices: string[];
	tree: ModelNode;
	step: number;
	uninitialized?: number;
	wrappers?: string[];
	path?: string;
	input?: string;
	output?: string;
	flops?: number;
	forward_error?: string;
	truncated?: boolean;
};

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
	/** Creating the first account takes the token printed in the server's log. */
	setup_token_required: boolean;
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

/** A page of the run list (`limit`/`after`), or the runs changed since `since_ms`. */
export type RunsPage = {
	runs: RunSummary[];
	/** Cursor of the next page, if there may be one. */
	next: string | null;
	/** Server time of the listing, for the next `since_ms`. */
	as_of_ms: number;
};

/** A tag of a project: how it shows, and how many runs have it (archived ones aside). */
export type TagInfo = {
	name: string;
	/** A palette color; null: derived from the name. */
	color: string | null;
	description: string;
	runs: number;
};
