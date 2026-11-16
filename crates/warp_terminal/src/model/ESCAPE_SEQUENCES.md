# Escape sequences

Escape sequence is a group (sequence) of characters that have a special meaning, usually different than the literal meaning of the characters used. 
In Warp we operate on ANSI escape codes, so they always have a form of:
`<ESC> <separating char> <some combination>`

The `<ESC>` is decimal 27 (0x1b) character. 
`<separating char>` is usually `[`, but can be something else too. The combination `<ESC><separating char>` is referred to as `C1 (8-Bit) Control Characters`.
The rest depends on the actual operation performed. May contain other characters and separators, and is defined on case-by-case basis. Some example operations include mouse tracking, moving the cursor in the apps or handling the special key combinations.

Note that this doc describes just the combinations **we (on behalf of user) write to pty** and doesn't include description of other sequences written by the applications or read from the shell. 


### Helpful reading
1. https://vt100.net/docs/vt100-ug/chapter3.html
2. https://www.xfree86.org/current/ctlseqs.html#PC-Style%20Function%20Keys
3. https://en.wikipedia.org/wiki/ANSI_escape_code


## C1 sequences - what are they and when do we use them?
List of all the C1 control characters [source](https://www.xfree86.org/current/ctlseqs.html#C1%20(8-Bit)%20Control%20Characters):

C1 sequence | Description
----------- | -----------
ESC D       | Index ( IND is 0x84)
ESC E       | Next Line ( NEL is 0x85)
ESC H       | Tab Set ( HTS is 0x88)
ESC M       | Reverse Index ( RI is 0x8d)
ESC N       | Single Shift Select of G2 Character Set ( SS2 is 0x8e): affects next character only
ESC O       | Single Shift Select of G3 Character Set ( SS3 is 0x8f): affects next character only
ESC P       | Device Control String ( DCS is 0x90)
ESC V       | Start of Guarded Area ( SPA is 0x96)
ESC W       | End of Guarded Area ( EPA is 0x97)
ESC X       | Start of String ( SOS is 0x98)
ESC Z       | Return Terminal ID (DECID is 0x9a). Obsolete form of CSI c (DA).
ESC [       | Control Sequence Introducer ( CSI is 0x9b)
ESC \       | String Terminator ( ST is 0x9c)
ESC ]       | Operating System Command ( OSC is 0x9d)
ESC ^       | Privacy Message ( PM is 0x9e)
ESC _       | Application Program Command ( APC is 0x9f)


So far, we're mostly using 2: CSI (ESC [) or SS3 (ESC O). Below there's a table that shows conditions for when to use each of those sequences:

| C1 sequence 	| terminal mode 	| modifiers (shift, ctrl, alt) 	| keys                                              	|
|-------------	|---------------	|------------------------------	|---------------------------------------------------	|
| CSI         	| Any           	| Optional                     	| Any                                               	|
| SS3         	| APP_CURSOR    	| Not used                     	| Arrow keys (up, down, right, left)<br>Home<br>End 	|

In short: `SS3` can only be used iff `TermMode::APP_CURSOR` is set && no modifiers were used and only for a certain group of keys. Otherwise, CSI is most likely the way to go.


## Use cases already covered in Warp

### Mouse tracking
Programs such as `vim` or `tmux` allow users to use the mouse within the app. There's couple modes of operations for mouse tracking (more [here](https://www.xfree86.org/current/ctlseqs.html#Mouse%20Tracking)), but the one we care about in Warp is `SGR`. 

Basically, some sort of low-res mouse tracking has been implemented before - it only allowed for tracking the mouse movement up to 223 columns, meaning, it wouldn't work in the bigger terminal window. As of 2012 xterm spec introduced `SGR`, which is supposed to support 'higher resolution' mouse tracking. Each of those modes expect different escape sequences to specify the mouse position, however, it is safe to assume that in modern world applications will favor SGR if supported by the terminal emulator, so we don't worry about the other sequences.

Below is the explanation of the sequences used:

`CSI < <button> ; <column> ; <row> ; <action>`

- `<button>` denotes the mouse button that was used. Left mouse button is 0, right one - 2, wheel has another number, dragging or pressing buttons with modifiers will have another number. As of now we only care about the Left mouse button and the Wheel and mouse dragging.
- `<column>` & `<row>` are basically coordinates of the mouse pointer at the moment of performing action.
- `<action>` can have 2 values: `M` for pressing and dragging; `m` for releasing the button.

Note that dragging is essentially *pressing a drag mouse button*.

### Cursor movement (with keyboard)
Regular cursor movement within the terminal - **unmodified** arrows and home/end key press actions - behave differently depending on the terminal mode. The terminal mode is set based on the program Warp is running, for example, long running command such as `vim` or `emacs` will set the `APP_CURSOR` mode (it's set using CSI ? 1h and unset with CSI ? 1l sequences). Warp keeps track of the mode in terminal_model (`is_term_mode_set` method can be of help).

|                            	| Normal mode 	| APP_CURSOR mode 	|
|----------------------------	|-------------	|-----------------	|
| Previous line (arrow up)   	| CSI A       	| SS3 A           	|
| Next line (arrow down)     	| CSI B       	| SS3 B           	|
| Next char (arrow right)    	| CSI C       	| SS3 C           	|
| Previous char (arrow left) 	| CSI D       	| SS3 D           	|
| First line (home)          	| CSI H       	| SS3 H           	|
| Last line (end)            	| CSI F       	| SS3 F           	|


### All the special keys and modifiers
Function keys? Function keys with Shift? Arrow with Meta or Alt? Shift + CMD + Key?
Unless we explicitly specified the binding somewhere in the `app/src/` code with a custom operation, then it should be handled by a proper escape sequence. This is a work in progress (and so is this README). Each of such sequences starts with `CSI` sequence, followed by proper combinations.

If modifiers are at play, below is the table with the values that should be used:

| Code 	| Modifier           	|
|------	|--------------------	|
| 2    	| Shift              	|
| 3    	| Alt                	|
| 4    	| Shift + Alt        	|
| 5    	| Ctrl               	|
| 6    	| Ctrl + Shift       	|
| 7    	| Ctrl + Alt         	|
| 8    	| Ctrl + Shift + Alt 	|

For example, sequence for arrows with modifiers has the following pattern:
`CSI 1 ; <modifier> <arrow code>`

(TODO: where exactly does `1 ;` come from?)

Other key combinations can have different values or completely different format. Best to follow the reading materials linked above to determine the right sequence. 
