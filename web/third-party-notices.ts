// Writes THIRD-PARTY-NOTICES.txt into the build: the licenses of what actually ends up in
// the bundle, read from the packages themselves, since minifying removes the comments that
// carry them. A license outside the allowed ones fails the build.

import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import type { Plugin } from 'vite';

/** Permissive: use, modify and redistribute, closed or open, with attribution at most. */
const ALLOWED = new Set([
	'0BSD',
	'Apache-2.0',
	'BSD-2-Clause',
	'BSD-3-Clause',
	'CC0-1.0',
	'ISC',
	'MIT',
	'OFL-1.1',
	'Unlicense'
]);
/** Bundled without being imported from JavaScript: fonts and styles through CSS. */
const ALSO_BUNDLED = ['@fontsource-variable/cascadia-mono', 'tailwindcss'];
const LICENSE_FILE = /^(licen[cs]e|copying|notice)/i;
const PACKAGE = /^(.*[/\\]node_modules[/\\](?:@[^/\\]+[/\\])?[^/\\]+)[/\\]/;

/** The folder of the package a module is in: nested copies are other versions of it. */
function packageFolder(id: string): string | null {
	return PACKAGE.exec(id.split('?')[0])?.[1] ?? null;
}

/** License expressions: OR needs one side allowed, AND needs both. */
function allowed(expression: string): boolean {
	const clean = expression.replace(/[()]/g, ' ');
	return clean
		.split(/\s+OR\s+/)
		.some((alternative) => alternative.split(/\s+AND\s+/).every((l) => ALLOWED.has(l.trim())));
}

/** Where the data of the password strength estimate comes from: see scripts/. */
const DERIVED_FROM = [
	{
		name: 'zxcvbn (word lists)',
		version: '4.4.2',
		license: 'MIT',
		repository: 'https://github.com/dropbox/zxcvbn'
	}
];

/** A link to its source, if it can be made into a plain https one. */
function repositoryOf(manifest: { repository?: string | { url?: string } }): string | null {
	const { repository } = manifest;
	const url = typeof repository === 'string' ? repository : repository?.url;
	if (!url) return null;
	const clean = url
		.replace(/^git\+/, '')
		.replace(/^git:\/\//, 'https://')
		.replace(/^ssh:\/\/git@/, 'https://')
		.replace(/^github:/, 'https://github.com/')
		.replace(/\.git$/, '');
	if (clean.startsWith('https://')) return clean;
	return /^[\w.-]+\/[\w.-]+$/.test(clean) ? `https://github.com/${clean}` : null;
}

/** A package without a `license` field can still say so in its license file. */
function licenseOf(manifest: { license?: string }, texts: string[]): string {
	if (manifest.license) return manifest.license;
	return texts.some((text) => /Permission is hereby granted, free of charge/.test(text))
		? 'MIT'
		: 'unknown';
}

export function thirdPartyNotices(): Plugin {
	const root = import.meta.dirname;
	return {
		name: 'losscape-third-party-notices',
		apply: 'build',
		generateBundle(_options, bundle) {
			if (this.environment?.name !== 'client') return;
			const folders = new Set(ALSO_BUNDLED.map((name) => join(root, 'node_modules', name)));
			for (const output of Object.values(bundle)) {
				if (output.type !== 'chunk') continue;
				for (const [id, module] of Object.entries(output.modules)) {
					const folder = packageFolder(id);
					if (folder && module.renderedLength > 0) folders.add(folder);
					// Brand icons are compiled in by unplugin-icons, under virtual module names.
					const icons = /~icons\/([^/]+)\//.exec(id);
					if (icons) folders.add(join(root, 'node_modules/@iconify-json', icons[1]));
				}
			}

			// The same package in several versions: once per version.
			const packages = new Map<
				string,
				{
					label: string;
					name: string;
					version: string;
					license: string;
					repository: string | null;
					texts: string[];
				}
			>();
			const refused: string[] = [];
			for (const folder of folders) {
				const manifest = JSON.parse(readFileSync(join(folder, 'package.json'), 'utf8'));
				const texts = readdirSync(folder)
					.filter((file) => LICENSE_FILE.test(file))
					.map((file) => readFileSync(join(folder, file), 'utf8').trim());
				const license = licenseOf(manifest, texts);
				const label = `${manifest.name} ${manifest.version}`;
				if (!allowed(license)) refused.push(`${label}: ${license}`);
				packages.set(label, {
					label,
					name: manifest.name,
					version: manifest.version,
					license,
					repository: repositoryOf(manifest),
					texts
				});
			}
			if (refused.length) {
				this.error(`Licenses not allowed in the web bundle:\n${refused.join('\n')}`);
			}

			const sorted = [...packages.values()].sort((a, b) => a.label.localeCompare(b.label));
			const sections = sorted.map(({ label, license, texts }) =>
				['='.repeat(80), `${label} (${license})`, '='.repeat(80), '', ...texts, ''].join('\n')
			);
			const derived = readFileSync(join(root, 'third-party-notices-data.txt'), 'utf8');
			this.emitFile({
				type: 'asset',
				fileName: 'THIRD-PARTY-NOTICES.txt',
				source: [
					"losscape's web app is built with these open source projects.\n",
					derived,
					...sections
				].join('\n')
			});
			// The same, as data for the About page.
			const own = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
			this.emitFile({
				type: 'asset',
				fileName: 'third-party-web.json',
				source: JSON.stringify({
					version: own.version,
					packages: [
						...DERIVED_FROM,
						...sorted.map(({ name, version, license, repository }) => ({
							name,
							version,
							license,
							repository
						}))
					]
				})
			});
		}
	};
}
