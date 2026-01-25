(*
 * motor_control.st - Motor start/stop with safety interlocks
 *
 * This program demonstrates a typical industrial motor control pattern
 * with start/stop buttons, emergency stop, and safety interlocks.
 *
 * Safety Philosophy:
 *   - Stop button and E-Stop use NC (normally closed) contacts
 *   - Loss of signal = safe state (motor stops)
 *   - Faults are latched until manually reset
 *
 * Variables represent I/O that would be mapped to hardware:
 *   Inputs: start_button, stop_button, e_stop, overload_ok, guard_closed
 *   Outputs: motor_contactor, run_indicator, fault_indicator
 *)

PROGRAM MotorControl
VAR
    (* Inputs - would be mapped to physical I/O *)
    start_button : BOOL := FALSE;   (* Momentary NO - TRUE when pressed *)
    stop_button : BOOL := TRUE;     (* NC - FALSE when pressed (fail-safe) *)
    e_stop : BOOL := TRUE;          (* NC - FALSE when pressed (fail-safe) *)
    overload_ok : BOOL := TRUE;     (* NC - FALSE when tripped *)
    guard_closed : BOOL := TRUE;    (* NO - TRUE when guard is closed *)

    (* Outputs - would be mapped to physical I/O *)
    motor_contactor : BOOL := FALSE;
    run_indicator : BOOL := FALSE;
    fault_indicator : BOOL := FALSE;

    (* Internal state *)
    motor_running : BOOL := FALSE;
    fault_latched : BOOL := FALSE;

    (* Restart delay counter (cycles) - prevents rapid restart *)
    restart_delay_counter : INT := 0;
    restart_delay_target : INT := 200;  (* 2 seconds at 10ms cycle *)
    restart_allowed : BOOL := FALSE;

    (* Composite safety status *)
    safety_ok : BOOL := FALSE;
END_VAR

(* Check all safety interlocks - all must be TRUE for safe operation *)
safety_ok := stop_button AND e_stop AND overload_ok AND guard_closed;

(* Fault detection - latch any fault condition that occurs while running *)
IF NOT safety_ok AND motor_running THEN
    fault_latched := TRUE;
END_IF;

(* Reset fault when conditions are safe and motor is stopped and start not pressed *)
IF NOT motor_running AND safety_ok AND NOT start_button THEN
    fault_latched := FALSE;
END_IF;

(* Restart delay - count up when motor stopped, reset when running *)
IF motor_running THEN
    restart_delay_counter := 0;
    restart_allowed := FALSE;
ELSE
    IF restart_delay_counter < restart_delay_target THEN
        restart_delay_counter := restart_delay_counter + 1;
        restart_allowed := FALSE;
    ELSE
        restart_allowed := TRUE;
    END_IF;
END_IF;

(* Motor start/stop logic - seal-in circuit pattern *)
IF motor_running THEN
    (* Stop conditions: loss of any safety interlock *)
    IF NOT safety_ok THEN
        motor_running := FALSE;
    END_IF;
ELSE
    (* Start conditions: start button + all interlocks + no fault + delay complete *)
    IF start_button AND safety_ok AND NOT fault_latched AND restart_allowed THEN
        motor_running := TRUE;
    END_IF;
END_IF;

(* Update outputs *)
motor_contactor := motor_running;
run_indicator := motor_running;
fault_indicator := fault_latched;

END_PROGRAM
