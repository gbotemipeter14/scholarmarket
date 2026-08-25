"use client";

import { useEffect, useState } from "react";

const STORAGE_KEY = "scholarmarket-theme";
const LIGHT = "light";
const DARK = "dark";

function getSystemTheme() {
	if (typeof window === "undefined") {
		return LIGHT;
	}

	return window.matchMedia("(prefers-color-scheme: dark)").matches
		? DARK
		: LIGHT;
}

function getInitialTheme() {
	if (typeof window === "undefined") {
		return LIGHT;
	}

	const storedTheme = window.localStorage.getItem(STORAGE_KEY);
	if (storedTheme === LIGHT || storedTheme === DARK) {
		return storedTheme;
	}

	return getSystemTheme();
}

function applyTheme(theme) {
	if (typeof document === "undefined") {
		return;
	}

	document.documentElement.dataset.theme = theme;
	document.documentElement.style.colorScheme = theme;
}

/**
 * Manages the user's light/dark theme preference.
 *
 * Reads the initial theme from localStorage (falling back to the system
 * preference), persists changes, and applies the theme to the document root.
 *
 * @returns {{ theme: 'light' | 'dark', isDark: boolean, toggleTheme: () => void }} The current theme state and a toggle function.
 */
export function useThemePreference() {
	const [theme, setTheme] = useState(getInitialTheme);

	useEffect(() => {
		applyTheme(theme);
	}, [theme]);

	const toggleTheme = () => {
		const nextTheme = theme === DARK ? LIGHT : DARK;
		window.localStorage.setItem(STORAGE_KEY, nextTheme);
		setTheme(nextTheme);
	};

	return {
		theme,
		isDark: theme === DARK,
		toggleTheme,
	};
}

