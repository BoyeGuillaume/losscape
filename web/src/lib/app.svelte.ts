import { toast } from 'svelte-sonner';

// What this deployment calls itself and how it looks (see docs/DEPLOYMENT.md). Public, since
// the sign-in page shows it too.

type AppInfo = {
	name: string;
	version: string;
	protocol: number;
	favicon: string | null;
	auth_background: string | null;
	/** Checkpoints can be uploaded (the server has storage for them). */
	checkpoints: boolean;
};

export const app = $state<{ info: AppInfo | null; name: string }>({ info: null, name: '' });

export async function loadApp(): Promise<void> {
	try {
		const res = await fetch('/api/v1/app');
		if (res.ok) {
			const before = app.info?.version;
			app.info = (await res.json()) as AppInfo;
			app.name = app.info.name;
			if (before && before !== app.info.version) {
				toast.info(`The server was updated to ${app.info.version}`, {
					description: 'Reload the page to get the matching version of this interface.',
					duration: Infinity,
					action: { label: 'Reload', onClick: () => location.reload() }
				});
			}
			return;
		}
	} catch {
		// As if nothing was configured.
	}
	app.name ||= 'losscape';
}
