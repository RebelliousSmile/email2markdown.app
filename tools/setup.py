from setuptools import setup, find_packages
setup(
    name="email-to-python-tools",
    version="0.1.0",
    packages=find_packages(where="src"),
    package_dir={"": "src"},
    install_requires=[
        "python-frontmatter",
        "PyYAML",
        "anthropic",
        "scikit-learn",
        "ollama",
        'tomli>=2; python_version < "3.11"',
    ],
    python_requires=">=3.8",
)
