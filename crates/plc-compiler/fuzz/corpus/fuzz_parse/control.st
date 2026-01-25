PROGRAM Control
VAR
    i : INT := 0;
    x : INT;
END_VAR
    IF i > 10 THEN
        x := 1;
    ELSIF i < 0 THEN
        x := -1;
    ELSE
        x := 0;
    END_IF;

    CASE i OF
        0: x := 100;
        1, 2: x := 200;
    ELSE
        x := 999;
    END_CASE;

    FOR i := 0 TO 10 DO
        x := x + 1;
    END_FOR;

    WHILE i < 20 DO
        i := i + 1;
    END_WHILE;
END_PROGRAM
