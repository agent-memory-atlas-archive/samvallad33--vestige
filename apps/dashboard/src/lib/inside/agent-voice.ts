/**
 * What an agent said about the memory it opened in the dashboard. Set when an
 * agent (any MCP client) opens a memory with words to explain it; the memory
 * page shows them above the memory's own words while that memory is open.
 */
import { writable } from 'svelte/store';

export interface AgentVoice {
	/** The memory the words are about (exact id). */
	id: string;
	/** Who is speaking (the agent's name, or "Your agent"). */
	by: string;
	text: string;
	at: number;
}

export const agentVoice = writable<AgentVoice | null>(null);
