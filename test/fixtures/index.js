export const users = {
    creator: {
        _id: "user_creator_1",
        walletAddress: "0xCreatorWalletAddress1234567890",
        email: "creator@scholarmarket.test",
        fullName: "Alice Educator",
    },
    buyer: {
        _id: "user_buyer_1",
        walletAddress: "0xBuyerWalletAddress0987654321",
        email: "buyer@scholarmarket.test",
        fullName: "Bob Student",
    }
};

export const materials = {
    draft: {
        _id: "mat_draft_123",
        title: "Draft Macroeconomics Notes",
        description: "In-progress notes.",
        userAddress: users.creator.walletAddress,
        creatorWallet: users.creator.walletAddress,
        status: "draft",
        price: "10",
        asset: "XLM",
        storageKey: "https://scholarmarket.test/files/draft.pdf",
        quarantineState: "clean",
    },
    published: {
        _id: "mat_pub_456",
        title: "Published Microeconomics Notes",
        description: "Complete notes.",
        creatorWallet: users.creator.walletAddress,
        status: "published",
        price: "15",
        asset: "USDC",
        contractId: "C_SOROBAN_CONTRACT_ID_789",
        ipfsHash: "QmTestHash...",
    }
};