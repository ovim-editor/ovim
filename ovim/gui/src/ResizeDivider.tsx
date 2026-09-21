import { createSignal } from "solid-js";

/** An accessible vertical separator with pointer capture and keyboard resizing. */
export default function ResizeDivider(props: {
    label: string;
    value: number;
    minimum: number;
    maximum: number;
    defaultValue: number;
    onChange: (value: number) => void;
}) {
    let drag: { pointer: number; x: number; value: number } | undefined;
    const [dragging, setDragging] = createSignal(false);
    const change = (value: number) =>
        props.onChange(
            Math.round(Math.max(props.minimum, Math.min(value, props.maximum))),
        );
    const stop = () => {
        drag = undefined;
        setDragging(false);
    };

    return (
        <div
            class="resize-divider"
            classList={{ dragging: dragging() }}
            role="separator"
            aria-label={props.label}
            aria-orientation="vertical"
            aria-valuemin={props.minimum}
            aria-valuemax={props.maximum}
            aria-valuenow={props.value}
            aria-valuetext={`${props.value} pixels`}
            tabIndex={0}
            data-gui-native-control
            onDblClick={() => change(props.defaultValue)}
            onKeyDown={(event) => {
                const step = event.shiftKey ? 40 : 10;
                let next: number;
                switch (event.key) {
                    case "ArrowLeft":
                        next = props.value - step;
                        break;
                    case "ArrowRight":
                        next = props.value + step;
                        break;
                    case "Home":
                        next = props.minimum;
                        break;
                    case "End":
                        next = props.maximum;
                        break;
                    case "Enter":
                        next = props.defaultValue;
                        break;
                    default:
                        return;
                }
                event.preventDefault();
                event.stopPropagation();
                change(next);
            }}
            onPointerDown={(event) => {
                if (event.button !== 0) return;
                event.preventDefault();
                drag = {
                    pointer: event.pointerId,
                    x: event.clientX,
                    value: props.value,
                };
                event.currentTarget.setPointerCapture(event.pointerId);
                setDragging(true);
            }}
            onPointerMove={(event) => {
                if (drag?.pointer === event.pointerId)
                    change(drag.value + event.clientX - drag.x);
            }}
            onPointerUp={(event) => {
                if (drag?.pointer !== event.pointerId) return;
                event.currentTarget.releasePointerCapture(event.pointerId);
                stop();
            }}
            onPointerCancel={stop}
            onLostPointerCapture={stop}
        />
    );
}
