(*
 * state_machine.st - Sequential process control (batch mixing example)
 *
 * This program demonstrates a state machine pattern for sequential
 * process control, simulating a simple batch mixing operation:
 *
 * States:
 *   0 = IDLE     - Wait for start command
 *   1 = FILL     - Fill tank to high level
 *   2 = MIX      - Run mixer for specified time
 *   3 = HEAT     - Heat to target temperature
 *   4 = DRAIN    - Drain tank to low level
 *   5 = COMPLETE - Signal done, return to IDLE
 *
 * This pattern is common in batch processing, packaging, and assembly operations.
 *)

PROGRAM BatchMixer
VAR
    (* State constants *)
    STATE_IDLE : INT := 0;
    STATE_FILL : INT := 1;
    STATE_MIX : INT := 2;
    STATE_HEAT : INT := 3;
    STATE_DRAIN : INT := 4;
    STATE_COMPLETE : INT := 5;

    (* Current state *)
    current_state : INT := 0;

    (* Inputs - would be mapped to physical I/O *)
    start_batch : BOOL := FALSE;      (* Start command from HMI/button *)
    tank_low : BOOL := TRUE;          (* Low level sensor - TRUE when below low *)
    tank_high : BOOL := FALSE;        (* High level sensor - TRUE when above high *)
    temperature : INT := 200;         (* Temperature sensor 0-1000 = 0-100.0 C *)
    e_stop : BOOL := TRUE;            (* Emergency stop NC - FALSE when pressed *)

    (* Outputs - would be mapped to physical I/O *)
    fill_valve : BOOL := FALSE;
    drain_valve : BOOL := FALSE;
    mixer_motor : BOOL := FALSE;
    heater : BOOL := FALSE;
    batch_complete : BOOL := FALSE;
    state_output : INT := 0;          (* Current state for HMI display *)

    (* Process parameters *)
    mix_time_cycles : INT := 3000;    (* 30 seconds at 10ms cycle *)
    target_temp : INT := 750;         (* 75.0 C in scaled units *)
    temp_hysteresis : INT := 20;      (* 2.0 C hysteresis *)
    complete_delay : INT := 500;      (* 5 seconds at 10ms cycle *)

    (* Timer counters *)
    mix_counter : INT := 0;
    complete_counter : INT := 0;
END_VAR

(* Emergency stop handling - immediate safe shutdown *)
IF NOT e_stop THEN
    current_state := STATE_IDLE;
    fill_valve := FALSE;
    drain_valve := FALSE;
    mixer_motor := FALSE;
    heater := FALSE;
    batch_complete := FALSE;
    mix_counter := 0;
    complete_counter := 0;
END_IF;

(* State machine - CASE statement is ideal for sequential control *)
CASE current_state OF
    0: (* IDLE - Safe state, all outputs off *)
        fill_valve := FALSE;
        drain_valve := FALSE;
        mixer_motor := FALSE;
        heater := FALSE;
        batch_complete := FALSE;
        mix_counter := 0;
        complete_counter := 0;

        (* Start only if e-stop OK and tank is empty *)
        IF start_batch AND e_stop AND tank_low THEN
            current_state := STATE_FILL;
        END_IF;

    1: (* FILL - Open fill valve until high level *)
        fill_valve := TRUE;
        drain_valve := FALSE;

        IF tank_high THEN
            fill_valve := FALSE;
            current_state := STATE_MIX;
        END_IF;

    2: (* MIX - Run mixer for specified time *)
        mixer_motor := TRUE;
        mix_counter := mix_counter + 1;

        IF mix_counter >= mix_time_cycles THEN
            mix_counter := 0;
            current_state := STATE_HEAT;
        END_IF;

    3: (* HEAT - Heat with temperature control *)
        mixer_motor := TRUE;  (* Keep mixing while heating *)

        (* Bang-bang control with hysteresis *)
        IF temperature < target_temp - temp_hysteresis THEN
            heater := TRUE;
        ELSIF temperature >= target_temp THEN
            heater := FALSE;
            current_state := STATE_DRAIN;
        END_IF;

    4: (* DRAIN - Open drain valve until empty *)
        mixer_motor := FALSE;
        heater := FALSE;
        drain_valve := TRUE;

        IF tank_low THEN
            drain_valve := FALSE;
            current_state := STATE_COMPLETE;
        END_IF;

    5: (* COMPLETE - Signal done, delay, return to IDLE *)
        batch_complete := TRUE;
        complete_counter := complete_counter + 1;

        IF complete_counter >= complete_delay THEN
            complete_counter := 0;
            batch_complete := FALSE;
            current_state := STATE_IDLE;
        END_IF;
END_CASE;

(* Output current state for HMI display *)
state_output := current_state;

END_PROGRAM
