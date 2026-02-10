/**
 * CC-Switch API Service
 *
 * Handles CC-Switch configuration import functionality with the Tauri backend.
 */

import { invoke } from '@tauri-apps/api/core';

/**
 * Options for importing CC-Switch configuration
 */
export interface ImportOptions {
  /** Whether to import Claude providers */
  importClaude: boolean;
  /** Whether to import Codex providers */
  importCodex: boolean;
  /** Whether to import MCP servers */
  importMcp: boolean;
  /** Whether to skip duplicate entries */
  skipDuplicates: boolean;
}

/**
 * Statistics for imported providers by type
 */
export interface CcSwitchProviderStats {
  /** Number of Claude providers imported */
  claude: number;
  /** Number of Codex providers imported */
  codex: number;
  /** Number of Gemini providers imported */
  gemini: number;
}

/**
 * Result of CC-Switch configuration import operation
 */
export interface CcSwitchImportResult {
  /** Whether the import completed successfully */
  success: boolean;
  /** Human-readable message about the import result */
  message: string;
  /** Statistics for imported providers */
  providersImported: CcSwitchProviderStats;
  /** Number of MCP servers imported */
  mcpServersImported: number;
  /** List of errors encountered during import */
  errors: string[];
}

/**
 * Preview of CC-Switch configuration without importing
 */
export interface CcSwitchPreview {
  /** Version of the CC-Switch configuration file */
  version: string;
  /** Count of providers by type in the configuration */
  providerCounts: CcSwitchProviderStats;
  /** Number of MCP servers in the configuration */
  mcpCount: number;
  /** Whether the configuration can be imported */
  canImport: boolean;
  /** Error message if preview failed */
  error?: string;
}

/**
 * Import a CC-Switch configuration file
 * @param filePath - Path to the CC-Switch configuration file
 * @param options - Import options specifying what to import
 * @returns Import result with statistics and any errors
 */
export const importCcSwitchConfig = async (
  filePath: string,
  options: ImportOptions
): Promise<CcSwitchImportResult> => {
  return await invoke<CcSwitchImportResult>('import_cc_switch_config', {
    filePath,
    options,
  });
};

/**
 * Preview a CC-Switch configuration file without importing
 * @param filePath - Path to the CC-Switch configuration file
 * @returns Preview data including provider counts and MCP servers
 */
export const previewCcSwitchConfig = async (
  filePath: string
): Promise<CcSwitchPreview> => {
  return await invoke<CcSwitchPreview>('preview_cc_switch_config', {
    filePath,
  });
};
