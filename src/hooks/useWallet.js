'use client';

import { useContext } from 'react';

import { WalletContext } from '@/providers/WalletProvider';

/**
 * Provides access to the Stellar wallet context, including connection state,
 * address, balance information, and signing methods.
 *
 * Must be used within a `<WalletProvider>` component.
 *
 * @returns {{
 *   state: object,
 *   connect: () => Promise<void>,
 *   disconnect: () => Promise<void>,
 *   signTransaction: (xdr: string, opts?: { address?: string }) => Promise<string>,
 *   signAuthEntry: (entryXdr: string, opts?: { address?: string }) => Promise<string>,
 *   isConnected: boolean,
 *   address: string | null,
 *   balances: object,
 *   refreshBalances: () => Promise<void>
 * }} The wallet context value.
 * @throws {Error} If used outside of a `<WalletProvider>`.
 */
export function useWallet() {
  const ctx = useContext(WalletContext);
  if (!ctx) {
    throw new Error('useWallet must be used inside <WalletProvider>');
  }
  return ctx;
}
