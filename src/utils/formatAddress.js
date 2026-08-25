/**
 * Truncates a blockchain address (or any long string) by showing the first and
 * last characters separated by an ellipsis.
 *
 * @param {string} address - The full address string to truncate.
 * @param {number} [startChars=6] - Number of characters to keep from the start.
 * @param {number} [endChars=4] - Number of characters to keep from the end.
 * @returns {string} The truncated address, or an empty string if no address is provided.
 */
export function formatAddress(address, startChars = 6, endChars = 4) {
  if (!address) return '';
  if (address.length <= startChars + endChars) return address;

  return `${address.slice(0, startChars)}...${address.slice(-endChars)}`;
}

/**
 * Truncates a Stellar transaction hash for compact display.
 *
 * @param {string} hash - The full transaction hash to truncate.
 * @param {number} [startChars=8] - Number of characters to keep from the start.
 * @param {number} [endChars=6] - Number of characters to keep from the end.
 * @returns {string} The truncated hash.
 */
export function formatHash(hash, startChars = 8, endChars = 6) {
  return formatAddress(hash, startChars, endChars);
}

/**
 * Formats a numeric balance to a fixed number of decimal places.
 *
 * @param {number|string|null|undefined} balance - The balance value to format.
 * @param {number} [decimals=4] - The number of decimal places.
 * @returns {string} The formatted balance string, or "0" if no balance is provided.
 */
export function formatBalance(balance, decimals = 4) {
  if (!balance) return '0';
  const num = typeof balance === 'string' ? parseFloat(balance) : balance;
  return num.toFixed(decimals);
}
