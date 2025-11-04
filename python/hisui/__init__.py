"""
Hisui - Python wrapper for Recording Composition Tool
"""

import json
import subprocess
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional, Union
from contextlib import contextmanager


class HisuiError(Exception):
    """Error during Hisui execution"""
    pass


class Hisui:
    """
    Wrapper class for hisui command

    Example:
        with Hisui() as hisui:
            # Get recording file information
            info = hisui.inspect("input.webm")
            print(info)

            # Compose recording files
            hisui.compose(
                root_dir="/path/to/archive/RECORDING_ID/",
                layout_file="layout.jsonc",
                output_file="output.mp4"
            )

            # List available codecs
            codecs = hisui.list_codecs()
            print(codecs)
    """

    def __init__(self, binary_path: Optional[str] = None, verbose: bool = False):
        """
        Initialize Hisui wrapper

        Args:
            binary_path: Path to hisui binary (searches PATH if not specified)
            verbose: Enable verbose logging
        """
        # When installed via maturin, the binary should be in PATH
        self.binary_path = binary_path or "hisui"
        self.verbose = verbose
        self._temp_files: list[str] = []

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        # Clean up temporary files
        for temp_file in self._temp_files:
            if Path(temp_file).exists():
                Path(temp_file).unlink()
        self._temp_files.clear()

    def _run_command(self, args: List[str], capture_output: bool = True) -> Union[str, None]:
        """
        Execute hisui command

        Args:
            args: List of command-line arguments
            capture_output: Whether to capture command output

        Returns:
            Command output (if capture_output is True)

        Raises:
            HisuiError: If command execution fails
        """
        cmd = [self.binary_path]
        if self.verbose:
            cmd.append("--verbose")
        cmd.extend(args)

        try:
            result = subprocess.run(
                cmd,
                capture_output=capture_output,
                text=True,
                check=True
            )

            if capture_output:
                return result.stdout
            return None

        except subprocess.CalledProcessError as e:
            error_msg = f"Command failed with exit code {e.returncode}"
            if e.stderr:
                error_msg += f"\nError output:\n{e.stderr}"
            raise HisuiError(error_msg) from e
        except FileNotFoundError:
            raise HisuiError(f"hisui binary not found at: {self.binary_path}") from None

    def inspect(self, input_file: str, decode: bool = False) -> Dict[str, Any]:
        """
        Get recording file information

        Args:
            input_file: Input file path
            decode: Whether to perform decoding

        Returns:
            File information as dictionary
        """
        args = ["inspect", input_file]
        if decode:
            args.append("--decode")

        output = self._run_command(args)
        if output:
            return json.loads(output)
        return {}

    def list_codecs(self) -> Dict[str, Any]:
        """
        Get list of available codecs

        Returns:
            Codec information as dictionary
        """
        args = ["list-codecs"]
        output = self._run_command(args)
        if output:
            return json.loads(output)
        return {}

    def compose(
        self,
        root_dir: str,
        layout_file: Optional[str] = None,
        output_file: Optional[str] = None,
        stats_file: Optional[str] = None,
        openh264: Optional[str] = None,
        no_progress_bar: bool = False,
        thread_count: Optional[int] = None,
        **kwargs
    ) -> None:
        """
        Compose recording files

        Args:
            root_dir: Root directory for composition processing
            layout_file: Layout file path to use for composition
            output_file: Output file path (default: ROOT_DIR/output.mp4)
            stats_file: Path to save statistics JSON
            openh264: Path to OpenH264 shared library
            no_progress_bar: Hide composition progress
            thread_count: Number of worker threads
            **kwargs: Additional options
        """
        args = ["compose", root_dir]

        if layout_file:
            args.extend(["--layout-file", layout_file])
        if output_file:
            args.extend(["--output-file", output_file])
        if stats_file:
            args.extend(["--stats-file", stats_file])
        if openh264:
            args.extend(["--openh264", openh264])
        if no_progress_bar:
            args.append("--no-progress-bar")
        if thread_count:
            args.extend(["--thread-count", str(thread_count)])

        # Additional options
        for key, value in kwargs.items():
            key = key.replace("_", "-")
            if isinstance(value, bool):
                if value:
                    args.append(f"--{key}")
            elif value is not None:
                args.extend([f"--{key}", str(value)])

        self._run_command(args, capture_output=False)

    def vmaf(
        self,
        root_dir: str,
        layout_file: Optional[str] = None,
        reference_yuv_file: Optional[str] = None,
        **kwargs
    ) -> Dict[str, Any]:
        """
        Evaluate video encoding quality using VMAF

        Args:
            root_dir: Root directory for composition processing
            layout_file: Layout file path for composition
            reference_yuv_file: Reference YUV file path
            **kwargs: Additional options

        Returns:
            VMAF evaluation output as dictionary
        """
        args = ["vmaf", root_dir]

        if layout_file:
            args.extend(["--layout-file", layout_file])
        if reference_yuv_file:
            args.extend(["--reference-yuv-file", reference_yuv_file])

        # Additional options
        for key, value in kwargs.items():
            key = key.replace("_", "-")
            if isinstance(value, bool):
                if value:
                    args.append(f"--{key}")
            elif value is not None:
                args.extend([f"--{key}", str(value)])

        output = self._run_command(args)
        if output:
            return json.loads(output)
        return {}

    def pipeline(
        self,
        pipeline_file: str,
        **kwargs
    ) -> None:
        """
        Execute user-defined pipeline

        Args:
            pipeline_file: Pipeline configuration file path
            **kwargs: Additional options
        """
        args = ["pipeline", "--file", pipeline_file]

        # Additional options
        for key, value in kwargs.items():
            key = key.replace("_", "-")
            if isinstance(value, bool):
                if value:
                    args.append(f"--{key}")
            elif value is not None:
                args.extend([f"--{key}", str(value)])

        self._run_command(args, capture_output=False)

    def tune(
        self,
        root_dir: str,
        layout_file: Optional[str] = None,
        search_space_file: Optional[str] = None,
        **kwargs
    ) -> Dict[str, Any]:
        """
        Tune video encoding parameters using Optuna

        Args:
            root_dir: Root directory for tuning processing
            layout_file: Layout file path to use for tuning
            search_space_file: Path to search space definition JSON file
            **kwargs: Additional options

        Returns:
            Tuning results as dictionary
        """
        args = ["tune", root_dir]

        if layout_file:
            args.extend(["--layout-file", layout_file])
        if search_space_file:
            args.extend(["--search-space-file", search_space_file])

        # Additional options
        for key, value in kwargs.items():
            key = key.replace("_", "-")
            if isinstance(value, bool):
                if value:
                    args.append(f"--{key}")
            elif value is not None:
                args.extend([f"--{key}", str(value)])

        output = self._run_command(args)
        if output:
            return json.loads(output)
        return {}

    def create_layout_config(
        self,
        layout_type: str,
        regions: List[Dict[str, Any]]
    ) -> str:
        """
        Create layout configuration file

        Args:
            layout_type: Layout type
            regions: List of region configurations

        Returns:
            Path to the created configuration file
        """
        with tempfile.NamedTemporaryFile(
            mode='w',
            suffix='.json',
            delete=False
        ) as f:
            config = {
                "type": layout_type,
                "regions": regions
            }
            json.dump(config, f, indent=2)
            temp_path = f.name

        self._temp_files.append(temp_path)
        return temp_path


# Helper function for context manager
@contextmanager
def hisui(binary_path: Optional[str] = None, verbose: bool = False):
    """
    Hisui context manager

    Example:
        with hisui() as h:
            h.inspect("input.webm")
    """
    h = Hisui(binary_path=binary_path, verbose=verbose)
    try:
        yield h
    finally:
        h.__exit__(None, None, None)


__all__ = ["Hisui", "HisuiError", "hisui"]