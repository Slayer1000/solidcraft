//! A design can hold coordinate measuring plans (`cmm`): setups, each with its probe,
//! alignment (with its bores measured as cylinders or circles, and optionally the clamping plate beside the part as the primary plane), safety plane, plane strategy and the measurements and characteristics (circle,
//! cylinder and plane measurements with their strategies and picked points, distances, angles,
//! point groups (one or many points, checked along the normal or as coordinates), each with an optional safety plane) and its NC programs (postprocessor, plan name,
//! output folder, post settings, the characteristics each outputs and whether a setup sheet and a CAD model are written). Older designs need no transformation: without
//! `cmm` a design has no plans. The last release refuses a design saved with this (the newer
//! format number).
