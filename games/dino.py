#!/usr/bin/env python3
"""dino.py -- a CLI remake of Chrome's offline dinosaur runner.

Run it in any terminal (needs only the Python standard library):

    python3 games/dino.py

Controls:
    space / up arrow : jump (also starts the game)
    down arrow       : duck (hold it down)
    p                : pause / resume
    r                : restart after a crash
    q                : quit
"""

import curses
import os
import random
import time

FPS = 30.0
FRAME_DT = 1.0 / FPS

GRAVITY = 40.0          # rows / s^2
JUMP_VELOCITY = 26.0    # rows / s (upward)
START_SPEED = 16.0      # cols / s
MAX_SPEED = 44.0        # cols / s
SPEED_RAMP = 0.30       # cols / s gained per second
BIRD_MIN_SCORE = 250    # birds start spawning past this score
DUCK_HOLD = 0.25        # seconds a down-key press keeps the dino ducked
DINO_X = 6              # fixed column of the dino
MIN_WIDTH = 60
MIN_HEIGHT = 18

HIGHSCORE_FILE = os.path.expanduser("~/.cli_dino_highscore")

DINO_RUN = [
    [
        "          __ ",
        "         / _)",
        "  .-^^^-/ /  ",
        " /       /   ",
        "<__.|_|-|_|  ",
    ],
    [
        "          __ ",
        "         / _)",
        "  .-^^^-/ /  ",
        " /       /   ",
        "<__.|-|_|-|  ",
    ],
]

DINO_JUMP = [
    "          __ ",
    "         / _)",
    "  .-^^^-/ /  ",
    " /       /   ",
    "<__.\\_\\-\\_\\  ",
]

DINO_DEAD = [
    "          __ ",
    "         / x)",
    "  .-^^^-/ /  ",
    " /       /   ",
    "<__.|_|-|_|  ",
]

DINO_DUCK = [
    [
        "  ____________ __ ",
        " <_|_|_|_|_|_|/ _)",
    ],
    [
        "  ____________ __ ",
        " <_|-|_|-|_|-|/ _)",
    ],
]

# (minimum speed at which this cactus may spawn, sprite). Wider/taller
# clusters only appear once the game is fast enough that a well-timed
# jump can still clear them.
CACTI = [
    (
        0.0,
        [
            " | ",
            "\\|/",
            " | ",
        ],
    ),
    (
        20.0,
        [
            "  |  ",
            " \\|/ ",
            "  |  ",
            " \\|/ ",
        ],
    ),
    (
        20.0,
        [
            " |   | ",
            "\\|/ \\|/",
            " |   | ",
        ],
    ),
    (
        26.0,
        [
            "  |    |  ",
            " \\|/  \\|/ ",
            "  |    |  ",
            "  |   \\|/ ",
        ],
    ),
]

BIRD = [
    [
        "\\     ",
        " \\    ",
        " (o=)>",
    ],
    [
        "      ",
        " /    ",
        " (o=)>",
    ],
]

GROUND_CHARS = "____________.__,____-___________,______.____"


def sprite_size(sprite):
    return len(sprite), max(len(row) for row in sprite)


def sprites_collide(ax, ay, a_sprite, bx, by, b_sprite):
    """Character-level collision between two sprites at integer positions."""
    ah, aw = sprite_size(a_sprite)
    bh, bw = sprite_size(b_sprite)
    x0, x1 = max(ax, bx), min(ax + aw, bx + bw)
    y0, y1 = max(ay, by), min(ay + ah, by + bh)
    if x0 >= x1 or y0 >= y1:
        return False
    for y in range(y0, y1):
        row_a = a_sprite[y - ay]
        row_b = b_sprite[y - by]
        for x in range(x0, x1):
            ca = row_a[x - ax] if x - ax < len(row_a) else " "
            cb = row_b[x - bx] if x - bx < len(row_b) else " "
            if ca != " " and cb != " ":
                return True
    return False


def load_high_score():
    try:
        with open(HIGHSCORE_FILE, encoding="utf-8") as f:
            return int(f.read().strip() or 0)
    except (OSError, ValueError):
        return 0


def save_high_score(score):
    try:
        with open(HIGHSCORE_FILE, "w", encoding="utf-8") as f:
            f.write(str(score))
    except OSError:
        pass


class Obstacle:
    def __init__(self, x, frames, bottom_offset, animated):
        self.x = float(x)
        self.frames = frames
        self.bottom_offset = bottom_offset  # rows above the ground line
        self.animated = animated

    def sprite(self, clock):
        if not self.animated:
            return self.frames[0]
        return self.frames[int(clock * 6) % len(self.frames)]

    def width(self):
        return sprite_size(self.frames[0])[1]


class Game:
    IDLE, RUNNING, DEAD = range(3)

    def __init__(self):
        self.high_score = load_high_score()
        self.reset()

    def reset(self):
        self.state = Game.IDLE
        self.speed = START_SPEED
        self.distance = 0.0
        self.score = 0
        self.height = 0.0        # dino height above ground, in rows
        self.velocity = 0.0
        self.duck_until = 0.0
        self.clock = 0.0
        self.obstacles = []
        self.gap_left = 30.0

    # -- input ------------------------------------------------------------

    def jump(self):
        if self.state == Game.IDLE:
            self.state = Game.RUNNING
        if self.state == Game.RUNNING and self.height == 0.0:
            self.velocity = JUMP_VELOCITY

    def duck(self):
        if self.state == Game.RUNNING:
            self.duck_until = self.clock + DUCK_HOLD

    # -- state ------------------------------------------------------------

    def is_ducking(self):
        return self.height == 0.0 and self.clock < self.duck_until

    def dino_sprite(self):
        if self.state == Game.DEAD:
            return DINO_DEAD
        if self.height > 0.0:
            return DINO_JUMP
        if self.is_ducking():
            return DINO_DUCK[int(self.clock * 8) % 2]
        if self.state == Game.IDLE:
            return DINO_RUN[0]
        return DINO_RUN[int(self.clock * 8) % 2]

    def spawn_obstacle(self, width):
        if self.score >= BIRD_MIN_SCORE and random.random() < 0.3:
            # 0 = must jump, 3 = must duck (or well-timed jump), 6 = fly-over
            altitude = random.choice((0, 3, 6))
            self.obstacles.append(Obstacle(width + 2, BIRD, altitude, True))
        else:
            kinds = [c for min_speed, c in CACTI if self.speed >= min_speed]
            cactus = random.choice(kinds)
            self.obstacles.append(Obstacle(width + 2, [cactus], 0, False))
        # Distance until the next spawn, scaled so faster games stay fair.
        self.gap_left = self.speed * random.uniform(1.1, 2.2) + 18.0

    def update(self, dt, width, ground_row):
        self.clock += dt
        if self.state != Game.RUNNING:
            return

        self.speed = min(MAX_SPEED, self.speed + SPEED_RAMP * dt)
        step = self.speed * dt
        self.distance += step
        old_score = self.score
        self.score = int(self.distance * 0.5)
        if self.score // 100 > old_score // 100:
            curses.beep()

        # Dino physics.
        if self.height > 0.0 or self.velocity > 0.0:
            self.height += self.velocity * dt
            self.velocity -= GRAVITY * dt
            if self.height <= 0.0:
                self.height = 0.0
                self.velocity = 0.0

        # Obstacles.
        for obstacle in self.obstacles:
            obstacle.x -= step
        self.obstacles = [o for o in self.obstacles if o.x + o.width() > 0]
        self.gap_left -= step
        if self.gap_left <= 0.0:
            self.spawn_obstacle(width)

        # Collisions.
        dino = self.dino_sprite()
        dino_top = self.dino_top(ground_row)
        for obstacle in self.obstacles:
            sprite = obstacle.sprite(self.clock)
            top = ground_row - obstacle.bottom_offset - len(sprite)
            if sprites_collide(
                DINO_X, dino_top, dino, int(round(obstacle.x)), top, sprite
            ):
                self.state = Game.DEAD
                if self.score > self.high_score:
                    self.high_score = self.score
                    save_high_score(self.high_score)
                curses.flash()
                break

    def dino_top(self, ground_row):
        sprite = self.dino_sprite()
        return ground_row - int(round(self.height)) - len(sprite)


def draw_sprite(win, top, left, sprite, attr=0):
    max_y, max_x = win.getmaxyx()
    for dy, row in enumerate(sprite):
        y = top + dy
        if y < 0 or y >= max_y:
            continue
        for dx, ch in enumerate(row):
            x = left + dx
            if ch == " " or x < 0 or x >= max_x:
                continue
            try:
                win.addch(y, x, ch, attr)
            except curses.error:
                pass


def draw_ground(win, ground_row, distance, width, attr):
    offset = int(distance)
    pattern = GROUND_CHARS
    line = "".join(pattern[(offset + i) % len(pattern)] for i in range(width))
    try:
        win.addnstr(ground_row, 0, line, width - 1, attr)
    except curses.error:
        pass


def draw_centered(win, y, text, attr=0):
    _, width = win.getmaxyx()
    x = max(0, (width - len(text)) // 2)
    try:
        win.addnstr(y, x, text, width - x - 1, attr)
    except curses.error:
        pass


def main(stdscr):
    curses.curs_set(0)
    stdscr.nodelay(True)
    stdscr.timeout(0)

    color = {"dino": 0, "cactus": 0, "bird": 0, "hud": 0}
    if curses.has_colors():
        curses.start_color()
        curses.use_default_colors()
        curses.init_pair(1, curses.COLOR_GREEN, -1)
        curses.init_pair(2, curses.COLOR_YELLOW, -1)
        curses.init_pair(3, curses.COLOR_CYAN, -1)
        color = {
            "dino": curses.color_pair(3),
            "cactus": curses.color_pair(1),
            "bird": curses.color_pair(2),
            "hud": curses.A_DIM,
        }

    game = Game()
    last = time.monotonic()

    while True:
        now = time.monotonic()
        dt = min(now - last, 4 * FRAME_DT)
        last = now

        # Drain input.
        while True:
            key = stdscr.getch()
            if key == -1:
                break
            if key in (ord("q"), ord("Q")):
                return
            if key == curses.KEY_RESIZE:
                stdscr.erase()
            elif key in (ord(" "), curses.KEY_UP, ord("w"), ord("W")):
                game.jump()
            elif key in (curses.KEY_DOWN, ord("s"), ord("S")):
                game.duck()
            elif key in (ord("p"), ord("P")) and game.state == Game.RUNNING:
                draw_centered(stdscr, 2, " PAUSED -- press any key ", curses.A_REVERSE)
                stdscr.refresh()
                stdscr.nodelay(False)
                if stdscr.getch() in (ord("q"), ord("Q")):
                    stdscr.nodelay(True)
                    return
                stdscr.nodelay(True)
                last = time.monotonic()
            elif key in (ord("r"), ord("R")) and game.state == Game.DEAD:
                game.reset()

        height, width = stdscr.getmaxyx()
        if width < MIN_WIDTH or height < MIN_HEIGHT:
            stdscr.erase()
            draw_centered(stdscr, height // 2, f"Need at least {MIN_WIDTH}x{MIN_HEIGHT} -- resize me!")
            stdscr.refresh()
            time.sleep(FRAME_DT)
            continue

        ground_row = height - 3
        game.update(dt, width, ground_row)

        # Draw.
        stdscr.erase()
        draw_ground(stdscr, ground_row, game.distance, width, color["hud"])
        for obstacle in game.obstacles:
            sprite = obstacle.sprite(game.clock)
            top = ground_row - obstacle.bottom_offset - len(sprite)
            attr = color["bird"] if obstacle.animated else color["cactus"]
            draw_sprite(stdscr, top, int(round(obstacle.x)), sprite, attr)
        draw_sprite(stdscr, game.dino_top(ground_row), DINO_X, game.dino_sprite(), color["dino"])

        hud = f"HI {game.high_score:05d}  {game.score:05d} "
        try:
            stdscr.addnstr(1, max(0, width - len(hud) - 2), hud, width - 1, color["hud"])
        except curses.error:
            pass

        if game.state == Game.IDLE:
            draw_centered(stdscr, height // 3, "* CLI DINO *", curses.A_BOLD)
            draw_centered(stdscr, height // 3 + 2, "press SPACE to start -- UP jumps, DOWN ducks, Q quits")
        elif game.state == Game.DEAD:
            draw_centered(stdscr, height // 3, " G A M E   O V E R ", curses.A_REVERSE | curses.A_BOLD)
            draw_centered(stdscr, height // 3 + 2, "press R to restart or Q to quit")

        stdscr.refresh()

        elapsed = time.monotonic() - now
        if elapsed < FRAME_DT:
            time.sleep(FRAME_DT - elapsed)


if __name__ == "__main__":
    try:
        curses.wrapper(main)
    except KeyboardInterrupt:
        pass
