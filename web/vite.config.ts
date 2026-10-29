import tailwindcss from '@tailwindcss/vite';
import Icons from 'unplugin-icons/vite';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vite';

// In dev, API calls go to the Rust server.
const server = process.env.LOSSCAPE_SERVER ?? 'http://localhost:8080';

export default defineConfig({
	server: { proxy: { '/api': server, '/healthz': server } },
	plugins: [
		tailwindcss(),
		// Brand icons (Iconify sets), compiled into components at build time: no runtime fetch.
		Icons({ compiler: 'svelte' }),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			// Single-page app served by the Rust server (unknown paths get index.html).
			adapter: adapter({ fallback: 'index.html' })
		})
	]
});
