'use client';

/**
 * Provides access to the toast notification system.
 *
 * Must be used within a `<ToastProvider>`.
 *
 * @returns {{
 *   show: (options: { id?: string, title?: string, message?: string, type?: 'info'|'success'|'error'|'loading', duration?: number }) => string,
 *   update: (id: string, updates: { title?: string, message?: string, type?: string, duration?: number }) => void,
 *   dismiss: (id: string) => void,
 *   toasts: Array<object>
 * }} The toast context API.
 * @throws {Error} If used outside of a `<ToastProvider>`.
 */
export { useToast } from '@/providers/ToastProvider';
