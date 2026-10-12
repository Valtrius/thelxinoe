import { touchDevice } from './responsive';

export function longPress(node: HTMLElement, onPress: () => void) {
  let callback = onPress;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pointer: { id: number; x: number; y: number } | undefined;
  let pressed = false;
  function clearClickSuppression() {
    window.removeEventListener('click', click, true);
    window.removeEventListener('pointerdown', nextPointer, true);
  }

  function nextPointer(event: PointerEvent) {
    if (pointer?.id !== event.pointerId) clearClickSuppression();
  }

  function cancel() {
    clearTimeout(timer);
    timer = undefined;
    pointer = undefined;
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', release);
    window.removeEventListener('pointercancel', cancel);
  }

  function trigger() {
    if (!pointer || pressed) return;
    clearTimeout(timer);
    timer = undefined;
    pressed = true;
    window.addEventListener('click', click, true);
    window.addEventListener('pointerdown', nextPointer, true);
    callback();
  }

  function down(event: PointerEvent) {
    if (
      !touchDevice.current ||
      event.pointerType === 'mouse' ||
      !event.isPrimary ||
      event.button !== 0 ||
      (event.target as Element).closest(
        '[data-long-press-ignore], input, textarea, select, a',
      )
    )
      return;
    cancel();
    pressed = false;
    clearClickSuppression();
    pointer = { id: event.pointerId, x: event.clientX, y: event.clientY };
    timer = setTimeout(trigger, 500);
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', release);
    window.addEventListener('pointercancel', cancel);
  }

  function move(event: PointerEvent) {
    if (
      pointer?.id === event.pointerId &&
      Math.hypot(event.clientX - pointer.x, event.clientY - pointer.y) > 10
    )
      cancel();
  }

  function release(event: PointerEvent) {
    if (pointer?.id !== event.pointerId) return;
    cancel();
  }

  function click(event: MouseEvent) {
    clearClickSuppression();
    pressed = false;
    event.preventDefault();
    event.stopImmediatePropagation();
  }

  function context(event: MouseEvent) {
    if (!touchDevice.current) return;
    event.preventDefault();
    trigger();
  }

  node.addEventListener('pointerdown', down);
  node.addEventListener('contextmenu', context);
  return {
    update(next: () => void) {
      callback = next;
    },
    destroy() {
      cancel();
      clearClickSuppression();
      node.removeEventListener('pointerdown', down);
      node.removeEventListener('contextmenu', context);
    },
  };
}
