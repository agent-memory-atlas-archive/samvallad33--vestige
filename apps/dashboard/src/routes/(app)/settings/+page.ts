import type { PageLoad } from './$types';
import { redirectAbsorbed } from '$lib/absorbed-redirect';

// Absorbed by the System organ (see ABSORBED_ROUTES in os-routes.ts).
export const load: PageLoad = ({ url }) => redirectAbsorbed('/settings', url);
