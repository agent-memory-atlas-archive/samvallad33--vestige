<script lang="ts">
	import { goto } from '$app/navigation';
	import { onMount } from 'svelte';
	import { base } from '$app/paths';
	import { HOME_ROUTE } from '$lib/os-routes';
	import { TRACE_SEEN_KEY } from '$lib/trace/trace-data';

	// The VestigeOS home is the Palace (the spatial launcher), NOT /graph. Use the
	// base-safe path so /dashboard/ lands on /dashboard/palace — a bare
	// goto('/graph') escapes the configured base and 404s on the deployed site.
	//
	// A first visit opens Trace instead, so the first thing anyone sees is
	// their own day answered by exact handle, with its receipt. Trace marks
	// itself seen; storage can be blocked, in which case the Palace wins.
	onMount(() => {
		let seen = true;
		try {
			seen = localStorage.getItem(TRACE_SEEN_KEY) === '1';
		} catch {
			seen = true;
		}
		goto(`${base}${seen ? HOME_ROUTE : '/trace'}`, { replaceState: true });
	});
</script>
