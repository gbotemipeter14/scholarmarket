import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { purchaseService } from '@/services/purchaseService';
import { queryKeys } from '@/lib/query/queryKeys';

/**
 * Fetches the authenticated user's purchase history.
 *
 * @returns {object} A react-query result containing an array of past purchases.
 */
export function usePurchaseHistory() {
  return useQuery({
    queryKey: queryKeys.purchases.history(),
    queryFn: () => purchaseService.getPurchaseHistory(),
  });
}

/**
 * Checks whether a specific address has purchased entitlement to a given material.
 *
 * @param {string} materialId - The ID of the material to check.
 * @param {string} address - The wallet address to check entitlement for.
 * @returns {object} A react-query result containing the entitlement status.
 */
export function useCheckEntitlement(materialId, address) {
  return useQuery({
    queryKey: queryKeys.purchases.entitlement(materialId, address),
    queryFn: () => purchaseService.checkEntitlement(materialId, address),
    enabled: !!materialId && !!address,
  });
}

/**
 * Mutation hook that records a new material purchase.
 *
 * Invalidates the purchase history and relevant entitlement caches on success.
 *
 * @returns {object} A react-query mutation object. Call `mutate(purchaseData)` to record a purchase.
 */
export function useCreatePurchase() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: purchaseService.createPurchase,
    onSuccess: (data, variables) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.purchases.all });
      // Invalidate specific entitlement check if materialId is known
      if (variables.materialId) {
        queryClient.invalidateQueries({
          predicate: (query) =>
            Array.isArray(query.queryKey) &&
            query.queryKey[0] === 'purchases' &&
            query.queryKey[1] === 'entitlement' &&
            query.queryKey[2] === variables.materialId,
        });
      }
    },
  });
}

/**
 * Mutation hook that initiates an access request for a material.
 *
 * Invalidates the relevant entitlement cache on success.
 *
 * @returns {object} A react-query mutation object. Call `mutate({ materialId, ... })` to start the access request.
 */
export function useStartAccessRequest() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: purchaseService.startAccessRequest,
    onSuccess: (_data, variables) => {
      if (variables.materialId) {
        queryClient.invalidateQueries({
          predicate: (query) =>
            Array.isArray(query.queryKey) &&
            query.queryKey[0] === 'purchases' &&
            query.queryKey[1] === 'entitlement' &&
            query.queryKey[2] === variables.materialId,
        });
      }
    },
  });
}
