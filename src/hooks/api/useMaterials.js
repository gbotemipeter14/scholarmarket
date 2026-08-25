import { keepPreviousData, useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { materialService } from '@/services/materialService';
import { queryKeys } from '@/lib/query/queryKeys';

/**
 * Fetches paginated marketplace materials with optional filtering parameters.
 *
 * @param {object} [params={}] - Query parameters for filtering (e.g., category, search, sort).
 * @returns {object} A react-query result containing marketplace material listings.
 */
export function useMarketplaceMaterials(params = {}) {
  return useQuery({
    queryKey: queryKeys.materials.marketplace(params),
    queryFn: () => materialService.getMarketplaceMaterials(params),
    placeholderData: keepPreviousData,
    staleTime: 5 * 60 * 1000,
  });
}

/**
 * Fetches the currently trending materials on the marketplace.
 *
 * @param {object} [params={}] - Optional query parameters for filtering trending materials.
 * @returns {object} A react-query result containing trending material listings.
 */
export function useTrendingMaterials(params = {}) {
  return useQuery({
    queryKey: queryKeys.materials.trending(params),
    queryFn: () => materialService.getTrendingMaterials(params),
    staleTime: 10 * 60 * 1000,
  });
}


/**
 * Fetches the full details of a single material by its ID.
 *
 * @param {string} id - The material ID to fetch.
 * @returns {object} A react-query result containing the material detail.
 */
export function useMaterialDetail(id) {
  return useQuery({
    queryKey: queryKeys.materials.detail(id),
    queryFn: () => materialService.getMaterialDetail(id),
    enabled: !!id,
  });
}

/**
 * Fetches feedback and reviews for a specific material.
 *
 * @param {string} id - The material ID whose feedback to fetch.
 * @returns {object} A react-query result containing the material feedback data.
 */
export function useMaterialFeedback(id) {
  return useQuery({
    queryKey: queryKeys.materials.feedback(id),
    queryFn: () => materialService.getMaterialFeedback(id),
    enabled: !!id,
  });
}

/**
 * Fetches all materials uploaded by the currently authenticated user.
 *
 * @returns {object} A react-query result containing the user's materials.
 */
export function useUserMaterials() {
  return useQuery({
    queryKey: queryKeys.materials.all,
    queryFn: () => materialService.getUserMaterials(),
  });
}

/**
 * Mutation hook that uploads a file for a new material.
 *
 * @returns {object} A react-query mutation object. Call `mutate(file)` to upload.
 */
export function useUploadFile() {
  return useMutation({
    mutationFn: materialService.uploadFile,
  });
}

/**
 * Mutation hook that creates a new material listing.
 *
 * Invalidates the user materials and marketplace queries on success.
 *
 * @returns {object} A react-query mutation object. Call `mutate(materialData)` to create.
 */
export function useCreateMaterial() {
  const queryClient = useQueryClient();
  
  return useMutation({
    mutationFn: materialService.createMaterial,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.materials.all });
      queryClient.invalidateQueries({ queryKey: ['materials', 'marketplace'] });
    },
  });
}

/**
 * Mutation hook that obtains a download URL for a purchased material.
 *
 * @returns {object} A react-query mutation object. Call `mutate(id)` with the material ID to get the download URL.
 */
export function useDownloadMaterial() {
  return useMutation({
    mutationFn: (id) => materialService.getDownloadUrl(id),
  });
}

/**
 * Mutation hook that updates an existing material's metadata.
 *
 * Invalidates the user materials and marketplace queries on success.
 *
 * @returns {object} A react-query mutation object. Call `mutate({ id, data })` to update.
 */
export function useUpdateMaterial() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ id, data }) => materialService.updateMaterial(id, data),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.materials.all });
      queryClient.invalidateQueries({ queryKey: ['materials', 'marketplace'] });
    },
  });
}

/**
 * Mutation hook that submits user feedback or a review for a material.
 *
 * Invalidates the feedback, detail, and marketplace queries on success.
 *
 * @param {string} id - The material ID to submit feedback for.
 * @returns {object} A react-query mutation object. Call `mutate(feedbackData)` to submit.
 */
export function useSubmitMaterialFeedback(id) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (feedbackData) => materialService.submitMaterialFeedback(id, feedbackData),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.materials.feedback(id) });
      queryClient.invalidateQueries({ queryKey: queryKeys.materials.detail(id) });
      queryClient.invalidateQueries({ queryKey: ['materials', 'marketplace'] });
    },
  });
}
