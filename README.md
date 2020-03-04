Command-line Pandora client.

# Background

A panharmonicon is an [instrument invented in 1805](https://en.wikipedia.org/wiki/Panharmonicon)
by a friend of Beethoven.  It was similar to a player piano in that it could automatically play
specially programmed music, but had the capacity to imitate all instruments and sound effects,
including gunfire and cannon shots.

![panharmonicon](assets/panharmonicon.png?raw=true "Johann Nepomuk Mälzel's Panharmonicon" | width=256)

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
* User-editable JSON configuration file
* Default configuration file can be generated with a command line flag
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
* Add menubar with entries corresponding to most hotkeys
* Add keybinding configuration
* Add user-configurable themes
* Robustness against Pandora session errors, e.g.

  ```
  [panharmonicon::model] src/model.rs:***: Failed while fetching new playlist: Pandora connection error: Pandora API error: Pandora API Call Error (Insufficient Connectivity Error): An unexpected error occurred
  ```
