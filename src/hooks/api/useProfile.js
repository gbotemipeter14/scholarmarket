import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { profileService } from '@/services/profileService';
import { queryKeys } from '@/lib/query/queryKeys';

/**
 * Fetches a user's public profile by their Stellar wallet address.
 *
 * @param {string} address - The wallet address of the user whose profile to fetch.
 * @returns {object} A react-query result containing the user profile data.
 */
export function useUserProfile(address) {
  return useQuery({
    queryKey: queryKeys.profile.detail(address),
    queryFn: () => profileService.getProfile(address),
    enabled: !!address,
  });
}

/**
 * Mutation hook that creates a new user profile.
 *
 * Invalidates the profile detail cache on success.
 *
 * @returns {object} A react-query mutation object. Call `mutate(profileData)` to create.
 */
export function useCreateProfile() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: profileService.createProfile,
    onSuccess: (data) => {
      if (data.user?.walletAddress) {
        queryClient.invalidateQueries({ 
          queryKey: queryKeys.profile.detail(data.user.walletAddress) 
        });
      }
    },
  });
}

/**
 * Fetches the top creators on the marketplace.
 *
 * Uses a 15-minute stale time since the leaderboard changes infrequently.
 *
 * @returns {object} A react-query result containing an array of top creator profiles.
 */
export function useTopCreators() {
  return useQuery({
    queryKey: queryKeys.profile.top(),
    queryFn: () => profileService.getTopCreators(),
    staleTime: 15 * 60 * 1000,
  });
}

/**
 * Mutation hook that updates an existing user profile.
 *
 * Invalidates the profile detail cache on success.
 *
 * @returns {object} A react-query mutation object. Call `mutate(profileData)` to update.
 */
export function useUpdateProfile() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: profileService.updateProfile,
    onSuccess: (data) => {
      if (data.user?.walletAddress) {
        queryClient.invalidateQueries({ 
          queryKey: queryKeys.profile.detail(data.user.walletAddress) 
        });
      }
    },
  });
}

/**
 * Fetches aggregate dashboard statistics for the authenticated creator.
 *
 * Uses a 5-minute stale time.
 *
 * @returns {object} A react-query result containing dashboard stats (e.g., total sales, earnings).
 */
export function useDashboardStats() {
  return useQuery({
    queryKey: queryKeys.dashboard.stats(),
    queryFn: () => profileService.getDashboardStats(),
    staleTime: 5 * 60 * 1000,
  });
}

