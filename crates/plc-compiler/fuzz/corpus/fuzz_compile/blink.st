(*
 * blink.st - Simple LED blink example
 *
 * This program toggles a digital output at a configurable interval,
 * demonstrating basic timing logic in IEC 61131-3 Structured Text.
 *
 * In a real PLC, this would drive an indicator LED or pilot light.
 * Assuming a 10ms cycle time, this blinks at ~1 Hz (50 cycles on, 50 off).
 *
 * Note: This example uses cycle counting for timing. In production code,
 * you would typically use TON/TOF timer function blocks when available.
 *)

PROGRAM Blink
VAR
    (* Output to LED - would be mapped to hardware via I/O configuration *)
    led_output : BOOL := FALSE;

    (* Cycle counter for timing *)
    counter : INT := 0;

    (* Number of cycles for half-period (50 = 500ms at 10ms cycle) *)
    half_period : INT := 50;
END_VAR

(* Increment counter each scan cycle *)
counter := counter + 1;

(* When counter reaches half_period, toggle LED and reset *)
IF counter >= half_period THEN
    led_output := NOT led_output;
    counter := 0;
END_IF;

END_PROGRAM
