'use client';

/**
 * Provides access to the shopping cart context, including items, totals,
 * and checkout functionality for purchasing materials.
 *
 * Must be used within a `<CartProvider>`.
 *
 * @returns {{
 *   cartItems: Array<object>,
 *   isCartOpen: boolean,
 *   setIsCartOpen: (open: boolean) => void,
 *   addToCart: (material: object) => void,
 *   removeFromCart: (id: string) => void,
 *   clearCart: () => void,
 *   totals: { subtotal: number, estimatedFees: number, grandTotal: number, creatorSplit: number, platformSplit: number },
 *   checkout: (email?: string) => Promise<void>
 * }} The cart context value.
 * @throws {Error} If used outside of a `<CartProvider>`.
 */
export { useCart } from '@/providers/CartProvider';
