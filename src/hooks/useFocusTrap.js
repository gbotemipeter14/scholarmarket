import { useEffect, useRef } from 'react';

/**
 * Traps keyboard focus within a modal element while it is active.
 *
 * Handles Tab/Shift+Tab cycling through focusable elements, pressing Escape
 * to close the modal, and restoring focus to the previously focused element
 * on cleanup.
 *
 * @param {boolean} isActive - Whether the focus trap is currently active.
 * @param {() => void} onClose - Callback invoked when the user presses Escape.
 * @returns {React.RefObject} A ref to attach to the modal container element.
 */
export default function useFocusTrap(isActive, onClose) {
    const modalRef = useRef(null);
    const triggerRef = useRef(null);
    const onCloseRef = useRef(onClose);

    useEffect(() => {
        onCloseRef.current = onClose;
    }, [onClose]);

    useEffect(() => {
        if (isActive) {
            // Save the element that triggered the modal
            triggerRef.current = document.activeElement;

            const focusableElements = Array.from(
                modalRef.current.querySelectorAll(
                    'button, a[href], area[href], input, select, textarea, iframe, audio[controls], video[controls], [tabindex], [contenteditable], summary'
                )
            ).filter((el) => {
                if (
                    el.disabled ||
                    el.getAttribute('tabindex') === '-1' ||
                    el.hasAttribute('hidden') ||
                    el.getAttribute('aria-hidden') === 'true'
                ) {
                    return false;
                }
                const { display, visibility } = window.getComputedStyle(el);
                return display !== 'none' && visibility !== 'hidden';
            });
            const firstElement = focusableElements[0];
            const lastElement = focusableElements[focusableElements.length - 1];

            const handleKeyDown = (e) => {
                if (e.key === 'Escape') {
                    onCloseRef.current();
                    e.preventDefault();
                }
                if (e.key === 'Tab') {
                    if (!focusableElements.length) {
                        e.preventDefault();
                        return;
                    }
                    if (e.shiftKey) {
                        if (document.activeElement === firstElement) {
                            lastElement.focus();
                            e.preventDefault();
                        }
                    } else {
                        if (document.activeElement === lastElement) {
                            firstElement.focus();
                            e.preventDefault();
                        }
                    }
                }
            };

            document.addEventListener('keydown', handleKeyDown);
            if (firstElement) firstElement.focus();

            return () => {
                document.removeEventListener('keydown', handleKeyDown);
                if (triggerRef.current) triggerRef.current.focus();
            };
        }
    }, [isActive]);

    return modalRef;
}
