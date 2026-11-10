pub(crate) fn fit_window(rect: [i32; 4], monitor: [i32; 4], work: [i32; 4]) -> [i32; 4] {
    let mut result = rect;
    for axis in 0..2 {
        let size = (rect[axis + 2] - rect[axis]).min(work[axis + 2] - work[axis]);
        let start = if rect[axis] < work[axis] || rect[axis + 2] > work[axis + 2] {
            work[axis] + (work[axis + 2] - work[axis] - size) / 2
        } else {
            rect[axis]
        };
        // SetWindowPlacement uses workspace, not screen, coordinates.
        result[axis] = start - (work[axis] - monitor[axis]);
        result[axis + 2] = result[axis] + size;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::fit_window;

    #[test]
    fn fits_and_centers_on_work_area_for_all_taskbar_sides_and_dpi() {
        for scale in [2, 3, 4] {
            let unit = |v| v * scale / 2;
            let monitor = [unit(-2560), unit(-1600), unit(0), unit(0)];
            for inset in [[0, 0, 0, 80], [0, 80, 0, 0], [80, 0, 0, 0], [0, 0, 80, 0]] {
                let work = [
                    monitor[0] + unit(inset[0]),
                    monitor[1] + unit(inset[1]),
                    monitor[2] - unit(inset[2]),
                    monitor[3] - unit(inset[3]),
                ];
                let oversized = [
                    monitor[0] - unit(100),
                    monitor[1] - unit(100),
                    monitor[2] + unit(100),
                    monitor[3] + unit(100),
                ];
                assert_eq!(
                    fit_window(oversized, monitor, work),
                    [
                        monitor[0],
                        monitor[1],
                        monitor[0] + work[2] - work[0],
                        monitor[1] + work[3] - work[1]
                    ]
                );
            }
        }
    }

    #[test]
    fn centers_an_oversize_axis_without_moving_a_valid_axis() {
        let monitor = [0, 0, 1280, 800];
        let work = [0, 0, 1280, 760];
        assert_eq!(
            fit_window([80, 0, 1200, 800], monitor, work),
            [80, 0, 1200, 760]
        );
        assert_eq!(
            fit_window([-20, 10, 480, 510], monitor, work),
            [390, 10, 890, 510]
        );
    }

    #[test]
    fn preserves_fitting_window_and_converts_workspace_coordinates() {
        let monitor = [-1000, -800, 0, 0];
        let work = [-960, -760, 0, 0];
        assert_eq!(
            fit_window([-900, -700, -400, -200], monitor, work),
            [-940, -740, -440, -240]
        );
    }
}
