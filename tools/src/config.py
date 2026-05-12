"""Configuration module for loading and managing application configuration.

This module provides robust configuration loading with validation and error handling.
"""

import sys
from pathlib import Path
from typing import Dict, Any, Optional

import yaml


def _validate_config_path(config_path: Path) -> None:
    """Validate that the configuration file exists and is accessible.
    
    Args:
        config_path: Path to the configuration file
        
    Raises:
        SystemExit: If the configuration file does not exist or is not accessible
    """
    if not config_path.exists():
        error_msg = f"Configuration file not found: {config_path}"
        print(f"Erreur: {error_msg}", file=sys.stderr)
        sys.exit(1)
    
    if not config_path.is_file():
        error_msg = f"Configuration path is not a file: {config_path}"
        print(f"Erreur: {error_msg}", file=sys.stderr)
        sys.exit(1)


def _load_yaml_content(file_path: Path) -> Optional[Dict[str, Any]]:
    """Load and parse YAML content from a file.
    
    Args:
        file_path: Path to the YAML file
        
    Returns:
        Parsed configuration dictionary, or None if file is empty
        
    Raises:
        yaml.YAMLError: If the YAML content is malformed
    """
    try:
        with file_path.open(encoding="utf-8") as fh:
            return yaml.safe_load(fh)
    except yaml.YAMLError as e:
        print(f"Failed to parse YAML file {file_path}: {e}", file=sys.stderr)
        raise


def load_config(config_path: Path) -> Dict[str, Any]:
    """Load configuration from a YAML file with validation and error handling.
    
    Args:
        config_path: Path to the YAML configuration file
        
    Returns:
        Dictionary containing the configuration data (never None)
        
    Raises:
        SystemExit: If the configuration file does not exist or is invalid
        yaml.YAMLError: If the YAML file is malformed
        
    Examples:
        >>> config = load_config(Path("config/config.yaml"))
        >>> print(config.get("database", {}).get("host"))
        
    Note:
        This function performs comprehensive validation including:
        - File existence check
        - File type verification
        - YAML parsing with error handling
        - Null-to-empty-dict conversion for consistent return type
    """
    _validate_config_path(config_path)
    
    config_data = _load_yaml_content(config_path)
    
    # Ensure we always return a dict, never None
    return config_data if config_data is not None else {}
