Command-line Pandora client.

# Background

A panharmonicon is an [instrument invented in 1805](https://en.wikipedia.org/wiki/Panharmonicon)
by a friend of Beethoven.  It was similar to a player piano in that it could automatically play
specially programmed music, but had the capacity to imitate all instruments and sound effects,
including gunfire and cannon shots.

<img src="https://raw.githubusercontent.com/compenguy/panharmonicon/master/assets/panharmonicon.png" width="256" height="256" alt="Johann Nepomuk Mälzel's Panharmonicon" title="Johann Nepomuk Mälzel's Panharmonicon">

# Screenshots

Scalable interface that provides full featureset in as little as 6 lines, and
intelligently scales interface elements down to 3 lines (although one line
works, too!).

![panharmonicon compact interface](assets/panharmonicon_mini_screenshot.png?raw=true "Compact interface")

On terminals that support it, allows mouse interactions with the interface.
Station select, volume control, pause, resume, skip to next song, rate tracks,
and remove track ratings all using your keyboard or mouse.

![panharmonicon mouse control](assets/panharmonicon_mouse_screenshot.png?raw=true "Mouse control")


# Features
* Visually select a station from a list
* Display current track, playback time, and volume
* Rating tracks (thumbs-up/down), and removing the rating from a track
* Support for caching tracks before playing them, providing robustness against network issues during playback
* Keybindings:

  | Key | Action |
  | --- | ------ |
  | q | Quit |
  | . | Pause |
  | > | Unpause |
  | p | Toggle pause |
  | ( | Volume down |
  | ) | Volume up |
  | n | Skip to next track |
  | t | Track is 'tired', suspend it for a month |
  | + | Thumbs-up track |
  | - | Thumbs-down track |
  | = | Clear track rating |

# TODO
* Add station editor (add/remove station seeds, directly edit track ratings, create/delete stations)
* Add help/about window
* Add message/info box
* Add keybinding configuration
* Add user-configurable themes
