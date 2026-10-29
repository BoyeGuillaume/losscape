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
	created_at_ms: number;
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

export type RunDetail = RunSummary & {
	project: string;
	notes: string;
	exit_code: number | null;
	environment: {
		hostname?: string;
		command?: string;
		python_version?: string;
		sdk_version?: string;
		git?: { remote?: string; commit?: string; dirty?: boolean };
	};
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

export type ProviderIconName = 'google' | 'github' | 'microsoft' | 'gitlab' | 'key';
