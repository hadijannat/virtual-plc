(*
 * pid_control.st - PID temperature control example
 *
 * This program demonstrates a basic PID (Proportional-Integral-Derivative)
 * controller for temperature regulation. It includes:
 *
 * - Configurable setpoint and PID gains
 * - Auto/Manual mode switching with bumpless transfer
 * - Output limiting (0-100%)
 * - Anti-windup for integral term (clamping + back-calculation)
 * - Derivative filtering to reduce noise sensitivity
 *
 * Scaling: All values use 0-10000 = 0-100.0 (one decimal place precision)
 * This integer-only implementation avoids floating-point for determinism.
 *)

PROGRAM TemperaturePID
VAR
    (* Inputs - would be mapped to analog I/O *)
    process_value : INT := 5000;      (* Temperature PV, 0-10000 = 0-100.0 C *)
    setpoint_input : INT := 5000;     (* Setpoint, 0-10000 = 0-100.0 C *)
    auto_mode : BOOL := TRUE;         (* TRUE = Auto, FALSE = Manual *)
    manual_output : INT := 0;         (* Manual output 0-10000 = 0-100% *)
    enable : BOOL := TRUE;            (* Controller enable *)

    (* Outputs - would be mapped to analog I/O *)
    control_output : INT := 0;        (* Output to heater, 0-10000 = 0-100% *)
    at_setpoint : BOOL := FALSE;      (* Within deadband of setpoint *)
    high_alarm : BOOL := FALSE;       (* Temperature high alarm *)
    low_alarm : BOOL := FALSE;        (* Temperature low alarm *)

    (* PID Parameters - scaled by 100 for integer math *)
    kp : INT := 200;      (* Proportional gain * 100 (2.0) *)
    ki : INT := 50;       (* Integral gain * 100 (0.5) *)
    kd : INT := 100;      (* Derivative gain * 100 (1.0) *)

    (* Output limits *)
    output_min : INT := 0;
    output_max : INT := 10000;

    (* Integral limits (anti-windup) *)
    integral_min : INT := -5000;
    integral_max : INT := 5000;

    (* Alarm thresholds *)
    high_alarm_sp : INT := 9000;   (* 90.0 C *)
    low_alarm_sp : INT := 1000;    (* 10.0 C *)
    deadband : INT := 50;          (* 0.5 C deadband for "at setpoint" *)

    (* Internal PID state *)
    error : INT := 0;
    error_prev : INT := 0;
    integral : INT := 0;
    derivative : INT := 0;
    derivative_filtered : INT := 0;

    (* Derivative filter coefficient (0-100, higher = more filtering) *)
    derivative_filter : INT := 80;

    (* Working variables *)
    pid_output : INT := 0;
    final_output : INT := 0;
    active_setpoint : INT := 5000;
END_VAR

(* Use external setpoint if valid, otherwise use default *)
IF setpoint_input >= 0 AND setpoint_input <= 10000 THEN
    active_setpoint := setpoint_input;
END_IF;

(* Calculate error: positive = need more heat *)
error := active_setpoint - process_value;

(* Check if controller is enabled *)
IF enable AND auto_mode THEN
    (* === PROPORTIONAL TERM === *)
    (* P = Kp * Error / 100 (done in final calculation) *)

    (* === INTEGRAL TERM with anti-windup === *)
    integral := integral + (ki * error / 1000);

    (* Clamp integral to prevent windup *)
    IF integral > integral_max THEN
        integral := integral_max;
    ELSIF integral < integral_min THEN
        integral := integral_min;
    END_IF;

    (* === DERIVATIVE TERM with filtering === *)
    (* Raw derivative (rate of change of error) *)
    derivative := error - error_prev;

    (* First-order low-pass filter reduces noise sensitivity *)
    (* Filtered = (filter * prev + (100-filter) * raw) / 100 *)
    derivative_filtered := (derivative_filter * derivative_filtered +
                           (100 - derivative_filter) * derivative) / 100;

    (* === CALCULATE PID OUTPUT === *)
    pid_output := (kp * error / 100) + integral + (kd * derivative_filtered / 100);

    (* Bias at 50% - typical for heating-only system *)
    pid_output := pid_output + 5000;

    (* === OUTPUT LIMITING with back-calculation anti-windup === *)
    IF pid_output > output_max THEN
        final_output := output_max;
        (* Back-calculate: undo integral contribution that caused saturation *)
        IF error > 0 THEN
            integral := integral - (ki * error / 1000);
        END_IF;
    ELSIF pid_output < output_min THEN
        final_output := output_min;
        IF error < 0 THEN
            integral := integral - (ki * error / 1000);
        END_IF;
    ELSE
        final_output := pid_output;
    END_IF;

    (* Store previous error for next derivative calculation *)
    error_prev := error;

ELSIF enable AND NOT auto_mode THEN
    (* === MANUAL MODE === *)
    IF manual_output >= output_min AND manual_output <= output_max THEN
        final_output := manual_output;
    END_IF;

    (* Reset integral for bumpless transfer back to auto *)
    integral := 0;
    error_prev := error;

ELSE
    (* === DISABLED - safe state === *)
    final_output := 0;
    integral := 0;
    error_prev := 0;
    derivative_filtered := 0;
END_IF;

(* Update output *)
control_output := final_output;

(* === STATUS OUTPUTS === *)
(* At setpoint if within deadband *)
IF error > -deadband AND error < deadband THEN
    at_setpoint := TRUE;
ELSE
    at_setpoint := FALSE;
END_IF;

(* Alarm outputs *)
high_alarm := process_value > high_alarm_sp;
low_alarm := process_value < low_alarm_sp;

END_PROGRAM
