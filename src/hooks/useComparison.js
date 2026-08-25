'use client';

/**
 * Provides access to the material comparison context, allowing users to add,
 * remove, and compare up to three materials side-by-side.
 *
 * Must be used within a `<ComparisonProvider>`.
 *
 * @returns {{
 *   comparedItems: Array<object>,
 *   isModalOpen: boolean,
 *   addToComparison: (material: object) => void,
 *   removeFromComparison: (id: string) => void,
 *   clearComparison: () => void,
 *   openComparisonModal: () => void,
 *   closeComparisonModal: () => void
 * }} The comparison context value.
 * @throws {Error} If used outside of a `<ComparisonProvider>`.
 */
export { useComparison } from '@/providers/ComparisonProvider';
