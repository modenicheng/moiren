//! Application-wide gpui actions: `Quit` exits the app from any window
//! (registered on the app context), while `Undo` and `Redo` name the graph
//! journal shortcuts the shell binds to ctrl-z / ctrl-y / ctrl-shift-z.
gpui_kit::actions!(moiren_actions, [Quit, Undo, Redo]);
